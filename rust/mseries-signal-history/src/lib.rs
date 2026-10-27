//! The Signal Pantheon client's message-history key, shared by `mseries-messages` and
//! `mseries-signal-import` so both stay byte-compatible with the client's own C store
//! (`signal_history_store_*`):
//!
//! * the 32-byte key is a base64 secret in the login keyring;
//! * `conversation_hash` = HMAC-SHA256(key, key-string);
//! * a row is AES-256-GCM with a random 12-byte nonce, no AAD, and a 16-byte tag kept apart
//!   from the ciphertext.
//!
//! The key lives in a [`Secret`], which is wiped on drop and never printed.

mod secret;

pub use secret::Secret;

use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::aead::{AeadInPlace, KeyInit, OsRng, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// One encrypted row body: nonce, ciphertext and tag are stored apart.
pub struct Sealed {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub tag: Vec<u8>,
}

/// The 32-byte history key. Never printed or logged; wiped when dropped.
pub struct HistoryKey(Secret);

impl HistoryKey {
    /// The keyring stores the key as base64 text.
    pub fn from_keyring_secret(secret: &[u8]) -> Result<Self, String> {
        let text: Vec<u8> = secret
            .iter()
            .copied()
            .filter(|byte| !byte.is_ascii_whitespace())
            .collect();
        let raw = Secret::new(
            base64::engine::general_purpose::STANDARD
                .decode(text)
                .map_err(|_| "the Signal history key is not valid base64".to_string())?,
        );
        Self::from_raw(raw.expose())
    }

    pub fn from_raw(raw: &[u8]) -> Result<Self, String> {
        if raw.len() != KEY_LEN {
            return Err("the Signal history key has the wrong length".into());
        }
        Ok(Self(Secret::new(raw.to_vec())))
    }

    /// `conversation_hash` for a key string.
    pub fn hash(&self, key_string: &str) -> Vec<u8> {
        let mut mac = <HmacSha256 as Mac>::new_from_slice(self.0.expose())
            .expect("HMAC accepts any key length");
        mac.update(key_string.as_bytes());
        mac.finalize().into_bytes().to_vec()
    }

    /// Encrypts with a fresh random nonce.
    pub fn encrypt(&self, plain: &[u8]) -> Result<Sealed, String> {
        let mut nonce = [0_u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        self.encrypt_with_nonce(&nonce, plain)
    }

    /// AES-256-GCM with the given nonce and no AAD; the tag is returned apart.
    pub fn encrypt_with_nonce(
        &self,
        nonce: &[u8; NONCE_LEN],
        plain: &[u8],
    ) -> Result<Sealed, String> {
        let cipher = Aes256Gcm::new(GenericArray::from_slice(self.0.expose()));
        let mut buffer = plain.to_vec();
        let tag = cipher
            .encrypt_in_place_detached(Nonce::from_slice(nonce), &[], &mut buffer)
            .map_err(|_| "could not encrypt a Signal history row".to_string())?;
        Ok(Sealed {
            nonce: nonce.to_vec(),
            ciphertext: buffer,
            tag: tag.to_vec(),
        })
    }

    pub fn decrypt(&self, nonce: &[u8], ciphertext: &[u8], tag: &[u8]) -> Result<Vec<u8>, String> {
        if nonce.len() != NONCE_LEN || tag.len() != TAG_LEN {
            return Err("a Signal history row has an unexpected shape".into());
        }
        let cipher = Aes256Gcm::new(GenericArray::from_slice(self.0.expose()));
        let mut buffer = ciphertext.to_vec();
        cipher
            .decrypt_in_place_detached(
                Nonce::from_slice(nonce),
                &[],
                &mut buffer,
                GenericArray::from_slice(tag),
            )
            .map_err(|_| "a Signal history row could not be authenticated".to_string())?;
        Ok(buffer)
    }
}

impl Clone for HistoryKey {
    fn clone(&self) -> Self {
        Self(Secret::new(self.0.expose().to_vec()))
    }
}

impl std::fmt::Debug for HistoryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HistoryKey(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];

    #[test]
    fn keyring_secret_must_be_a_32_byte_base64_key() {
        let good = base64::engine::general_purpose::STANDARD.encode(KEY);
        assert!(HistoryKey::from_keyring_secret(format!("{good}\n").as_bytes()).is_ok());
        assert!(HistoryKey::from_keyring_secret(b"not base64 !!").is_err());
        assert!(HistoryKey::from_keyring_secret(b"AAAA").is_err());
    }

    #[test]
    fn rows_round_trip_and_reject_tampering_or_another_key() {
        let key = HistoryKey::from_raw(&KEY).unwrap();
        let sealed = key.encrypt(b"hello").unwrap();
        assert_eq!(
            key.decrypt(&sealed.nonce, &sealed.ciphertext, &sealed.tag)
                .unwrap(),
            b"hello"
        );
        let mut bad = sealed.ciphertext.clone();
        bad[0] ^= 1;
        assert!(key.decrypt(&sealed.nonce, &bad, &sealed.tag).is_err());
        let other = HistoryKey::from_raw(&[9; 32]).unwrap();
        assert!(
            other
                .decrypt(&sealed.nonce, &sealed.ciphertext, &sealed.tag)
                .is_err()
        );
        assert!(
            key.decrypt(&sealed.nonce[..11], &sealed.ciphertext, &sealed.tag)
                .is_err()
        );
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Vectors computed independently (Python hmac / cryptography), so a change of backend or layout
    /// cannot silently break the on-disk format the Signal Pantheon client also reads.
    #[test]
    fn matches_the_client_format() {
        let key = HistoryKey::from_raw(&KEY).unwrap();
        assert_eq!(
            hex(&key.hash("pantheon-v1:+15550000001:catalog")),
            "b511ced32a1254e7ec65d0b1a3622ebc4897788ef75b2735918e7af27e9bc432"
        );
        let nonce: [u8; 12] = std::array::from_fn(|i| i as u8);
        let sealed = key.encrypt_with_nonce(&nonce, b"abc").unwrap();
        assert_eq!(hex(&sealed.ciphertext), "79e38a");
        assert_eq!(hex(&sealed.tag), "e5ff18ce4126c9b9f9762766e51de892");
    }

    #[test]
    fn clone_and_debug_keep_the_key_out_of_view() {
        let key = HistoryKey::from_raw(&KEY).unwrap();
        let copy = key.clone();
        assert_eq!(key.hash("a"), copy.hash("a"));
        assert_eq!(format!("{key:?}"), "HistoryKey(..)");
    }
}
