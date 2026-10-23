//! Chromium/Electron `safeStorage` decryption on Linux, as used by Signal Desktop
//! for the SQLCipher key in `config.json` (`encryptedKey`).
//!
//! Layout: `v11` (or `v10`) + AES-128-CBC ciphertext. The key is
//! PBKDF2-HMAC-SHA1(password, "saltysalt", 1 iteration, 16 bytes), the IV is 16
//! spaces and the padding is PKCS#7. `v11` takes its password from the keyring;
//! `v10` uses the fixed password `peanuts`.

use openssl::{
    hash::MessageDigest,
    pkcs5::pbkdf2_hmac,
    symm::{Cipher, decrypt},
};

const SALT: &[u8] = b"saltysalt";

pub fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return Err("odd-length hex".into());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| "invalid hex".to_string()))
        .collect()
}

pub fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether the blob needs the keyring password (`v11`) rather than the fixed one.
pub fn needs_keyring(encrypted: &[u8]) -> bool {
    encrypted.starts_with(b"v11")
}

pub fn decrypt_blob(encrypted: &[u8], keyring_password: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let password: &[u8] = match encrypted.get(..3) {
        Some(b"v11") => keyring_password.ok_or("the keyring password is required")?,
        Some(b"v10") => b"peanuts",
        _ => return Err("unrecognised encryptedKey prefix".into()),
    };
    let mut key = [0u8; 16];
    pbkdf2_hmac(password, SALT, 1, MessageDigest::sha1(), &mut key).map_err(|e| format!("key derivation failed: {e}"))?;
    decrypt(Cipher::aes_128_cbc(), &key, Some(&[b' '; 16]), &encrypted[3..])
        .map_err(|_| "could not decrypt encryptedKey (wrong keyring password?)".to_string())
}
