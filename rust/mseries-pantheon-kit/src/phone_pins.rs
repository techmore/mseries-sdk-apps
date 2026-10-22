//! Bounded Apple Messages pin snapshots, exact conversation matching and
//! phone-scoped encrypted preferences.

use crate::Thread;
use gtk::{gio, glib};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub const APPLE_PINS_SCHEMA: &str = "io.elementary.mseries.apple-message-pins";
const KEYRING_APPLICATION: &str = "io.elementary.mseries.Messages";
const MAX_IMPORT_BYTES: usize = 128 * 1024;
const MAX_PINS: usize = 32;
const MAX_APPLE_IDENTIFIER_BYTES: usize = 2048;
const MAX_THREAD_KEY_BYTES: usize = 4096;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct ApplePin {
    pub apple_identifier: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct PinSnapshot {
    pub schema: String,
    pub version: u64,
    pub source: String,
    pub captured_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_modified_at: Option<String>,
    pub pins: Vec<ApplePin>,
}

impl PinSnapshot {
    /// Read at most 128 KiB from a user-selected local export file.
    pub fn read(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|_| "Could not open that pin export.".to_string())?;
        let mut bytes = Vec::with_capacity(MAX_IMPORT_BYTES.min(16 * 1024));
        file.take((MAX_IMPORT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read that pin export.".to_string())?;
        Self::parse(&bytes)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_IMPORT_BYTES {
            return Err("The pin export is too large.".into());
        }
        let snapshot: Self = serde_json::from_slice(bytes)
            .map_err(|_| "The pin export could not be read.".to_string())?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema != APPLE_PINS_SCHEMA
            || self.version != 1
            || !matches!(self.source.as_str(), "macos-imcore" | "macos-pin-cache")
        {
            return Err("This is not a supported Apple Messages pin export.".into());
        }
        let captured = parse_utc_timestamp(&self.captured_at)
            .ok_or_else(|| "The pin export has an invalid capture time.".to_string())?;
        let now = glib::DateTime::now_utc()
            .map_err(|_| "The current time could not be read.".to_string())?
            .to_unix();
        if captured > now.saturating_add(300) {
            return Err("The pin export has an invalid capture time.".into());
        }
        if self.pins.len() > MAX_PINS {
            return Err("The pin export has an invalid conversation list.".into());
        }
        let mut seen = HashSet::with_capacity(self.pins.len());
        for pin in &self.pins {
            if pin.apple_identifier.is_empty()
                || pin.apple_identifier.len() > MAX_APPLE_IDENTIFIER_BYTES
                || pin.apple_identifier.chars().any(char::is_control)
                || !seen.insert(pin.apple_identifier.as_str())
            {
                return Err("The pin export has invalid or duplicate identifiers.".into());
            }
        }
        if let Some(modified_at) = &self.source_modified_at {
            let modified = parse_utc_timestamp(modified_at)
                .ok_or_else(|| "The Apple pin modification time is invalid.".to_string())?;
            if modified > captured.saturating_add(300) {
                return Err("The Apple pin modification time is invalid.".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct ImportedPinState {
    pub version: u64,
    pub snapshot: Option<PinSnapshot>,
    pub choices: Vec<Option<String>>,
}

impl Default for ImportedPinState {
    fn default() -> Self {
        Self {
            version: 1,
            snapshot: None,
            choices: Vec::new(),
        }
    }
}

impl ImportedPinState {
    pub fn reviewed(
        snapshot: PinSnapshot,
        choices: Vec<Option<String>>,
        threads: &[Thread],
    ) -> Result<Self, String> {
        snapshot.validate()?;
        let keys = threads
            .iter()
            .map(|thread| thread.key.as_str())
            .collect::<HashSet<_>>();
        if choices.len() != snapshot.pins.len() {
            return Err("Review every imported pin before continuing.".into());
        }
        let mut seen = HashSet::with_capacity(choices.len());
        for key in choices.iter().flatten() {
            if key.is_empty()
                || key.len() > MAX_THREAD_KEY_BYTES
                || !keys.contains(key.as_str())
                || !seen.insert(key.as_str())
            {
                return Err("Choose each conversation once. A conversation may have changed; review the matches.".into());
            }
        }
        Ok(Self {
            version: 1,
            snapshot: Some(snapshot),
            choices,
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("The saved pin import is unsupported.".into());
        }
        let Some(snapshot) = &self.snapshot else {
            return if self.choices.is_empty() {
                Ok(())
            } else {
                Err("The saved conversation matches are invalid.".into())
            };
        };
        snapshot.validate()?;
        if self.choices.len() != snapshot.pins.len() {
            return Err("The saved conversation matches are invalid.".into());
        }
        let mut seen = HashSet::with_capacity(self.choices.len());
        for key in self.choices.iter().flatten() {
            if key.is_empty() || key.len() > MAX_THREAD_KEY_BYTES || !seen.insert(key.as_str()) {
                return Err("The saved conversation matches are invalid.".into());
            }
        }
        Ok(())
    }

    /// Keep explicit group matches on reimport; auto-match only one unique
    /// individual conversation by exact normalized address.
    pub fn suggested_matches(
        &self,
        snapshot: &PinSnapshot,
        threads: &[Thread],
    ) -> Vec<Option<String>> {
        suggested_pin_matches(snapshot, threads, Some(self))
    }

    /// Apply the imported Apple order, then list all remaining conversations
    /// by their most recent message time.
    pub fn ordered_thread_keys(&self, threads: &[Thread]) -> Vec<String> {
        let current = threads
            .iter()
            .map(|thread| thread.key.as_str())
            .collect::<HashSet<_>>();
        let mut ordered = Vec::with_capacity(threads.len());
        let mut included = HashSet::with_capacity(threads.len());
        if let Some(snapshot) = &self.snapshot {
            for key in effective_choices(self, snapshot, threads)
                .into_iter()
                .flatten()
            {
                if current.contains(key.as_str()) && included.insert(key.clone()) {
                    ordered.push(key);
                }
            }
        }
        let mut remaining = threads
            .iter()
            .filter(|thread| !included.contains(thread.key.as_str()))
            .collect::<Vec<_>>();
        remaining.sort_by(|left, right| right.last_ts.cmp(&left.last_ts));
        ordered.extend(remaining.into_iter().map(|thread| thread.key.clone()));
        ordered
    }
}

pub fn suggested_pin_matches(
    snapshot: &PinSnapshot,
    threads: &[Thread],
    previous: Option<&ImportedPinState>,
) -> Vec<Option<String>> {
    let mut known = HashMap::new();
    if let Some(previous) = previous
        && let Some(old_snapshot) = &previous.snapshot
    {
        for (pin, key) in old_snapshot.pins.iter().zip(&previous.choices) {
            if let Some(key) = key
                && threads.iter().any(|thread| thread.key == *key)
            {
                known.insert(pin.apple_identifier.as_str(), key.clone());
            }
        }
    }
    snapshot
        .pins
        .iter()
        .map(|pin| {
            if let Some(key) = known.get(pin.apple_identifier.as_str()) {
                return Some(key.clone());
            }
            let address = normalized_address(&pin.apple_identifier)?;
            let matches = threads
                .iter()
                .filter(|thread| {
                    !thread.is_group
                        && thread.recipients.len() == 1
                        && normalized_address(&thread.recipients[0]).as_deref()
                            == Some(address.as_str())
                })
                .map(|thread| thread.key.clone())
                .take(2)
                .collect::<Vec<_>>();
            (matches.len() == 1).then(|| matches[0].clone())
        })
        .collect()
}

fn effective_choices(
    state: &ImportedPinState,
    snapshot: &PinSnapshot,
    threads: &[Thread],
) -> Vec<Option<String>> {
    let keys = threads
        .iter()
        .map(|thread| thread.key.as_str())
        .collect::<HashSet<_>>();
    let suggested = suggested_pin_matches(snapshot, threads, None);
    state
        .choices
        .iter()
        .enumerate()
        .map(|(index, choice)| match choice {
            Some(key)
                if !keys.contains(key.as_str())
                    && (key.starts_with("inbox:") || key.starts_with("new:")) =>
            {
                suggested.get(index).cloned().flatten()
            }
            other => other.clone(),
        })
        .collect()
}

fn normalized_address(value: &str) -> Option<String> {
    let candidate = value.trim();
    let bytes = candidate.as_bytes();
    let phone_like = !bytes.is_empty()
        && (bytes[0] != b'+' || bytes.len() > 1)
        && bytes.iter().enumerate().all(|(index, byte)| {
            byte.is_ascii_digit() || (index == 0 && *byte == b'+') || b" ()-.".contains(byte)
        });
    if phone_like {
        let digits = candidate.bytes().filter(u8::is_ascii_digit).count();
        let remainder = if candidate.starts_with('+') {
            &bytes[1..]
        } else {
            bytes
        };
        if (3..=20).contains(&digits)
            && remainder.len() >= 3
            && remainder.len() <= 30
            && remainder[0].is_ascii_digit()
        {
            return Some(format!(
                "{}{}",
                if candidate.starts_with('+') { "+" } else { "" },
                candidate
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect::<String>()
            ));
        }
    }
    if candidate.len() > 254 || candidate.matches('@').count() != 1 || !candidate.is_ascii() {
        return None;
    }
    let (local, domain) = candidate.split_once('@')?;
    if local.is_empty()
        || local.len() > 64
        || domain.is_empty()
        || domain.len() > 253
        || domain.starts_with('.')
        || domain.ends_with('.')
    {
        return None;
    }
    Some(candidate.to_ascii_lowercase())
}

fn parse_utc_timestamp(value: &str) -> Option<i64> {
    let time = value.split_once('T')?.1;
    if !time.ends_with('Z') && !time.contains('+') && !time.contains('-') {
        return None;
    }
    let parsed = glib::DateTime::from_iso8601(value, None).ok()?;
    Some(parsed.to_unix())
}

/// Uses the desktop Secret Service only. There is deliberately no plaintext
/// file fallback for imported phone identifiers or pin choices.
pub struct PhonePinStore;

impl PhonePinStore {
    pub async fn load(phone_profile_id: &str) -> Result<ImportedPinState, String> {
        validate_profile_id(phone_profile_id)?;
        let args = secret_attributes(phone_profile_id);
        let mut search = vec!["search".to_string()];
        search.extend(args.clone());
        let (search_status, search_output, _) = run_secret_tool(&search, None).await?;
        if search_status != 0 {
            return Err("Could not open the desktop keyring.".into());
        }
        if search_output.trim().is_empty() || search_output.contains("No items found") {
            return Ok(ImportedPinState::default());
        }

        let mut lookup = vec!["lookup".to_string()];
        lookup.extend(args);
        let (status, value, _) = run_secret_tool(&lookup, None).await?;
        if status != 0 {
            return Err("Could not read imported pins from the desktop keyring.".into());
        }
        if value.len() > MAX_IMPORT_BYTES {
            return Err("The saved pin import is too large.".into());
        }
        let state: ImportedPinState = serde_json::from_str(value.trim())
            .map_err(|_| "The saved pin import is invalid.".to_string())?;
        state.validate()?;
        Ok(state)
    }

    pub async fn save(phone_profile_id: &str, state: &ImportedPinState) -> Result<(), String> {
        validate_profile_id(phone_profile_id)?;
        state.validate()?;
        let value = serde_json::to_string(state)
            .map_err(|_| "The saved pin import could not be encoded.".to_string())?;
        if value.len() > MAX_IMPORT_BYTES {
            return Err("The saved pin import is too large.".into());
        }
        let mut args = vec![
            "store".to_string(),
            "--label=Messages — imported Apple conversation pins".to_string(),
        ];
        args.extend(secret_attributes(phone_profile_id));
        let (status, _, _) = run_secret_tool(&args, Some(&value)).await?;
        if status != 0 {
            return Err("Could not save imported pins in the desktop keyring.".into());
        }
        Ok(())
    }

    pub async fn clear(phone_profile_id: &str) -> Result<(), String> {
        validate_profile_id(phone_profile_id)?;
        let mut args = vec!["clear".to_string()];
        args.extend(secret_attributes(phone_profile_id));
        let (status, _, _) = run_secret_tool(&args, None).await?;
        if status != 0 {
            return Err("Could not remove imported pins from the desktop keyring.".into());
        }
        Ok(())
    }
}

fn secret_attributes(phone_profile_id: &str) -> Vec<String> {
    vec![
        "application".into(),
        KEYRING_APPLICATION.into(),
        "phone".into(),
        phone_profile_id.into(),
    ]
}

fn validate_profile_id(value: &str) -> Result<(), String> {
    if is_valid_phone_profile_id(value) {
        Ok(())
    } else {
        Err("BlueFerry does not report a valid paired-phone profile ID. Update BlueFerry and connect your phone.".into())
    }
}

pub fn is_valid_phone_profile_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23].iter().all(|index| bytes[*index] == b'-')
        && bytes.iter().enumerate().all(|(index, byte)| {
            [8, 13, 18, 23].contains(&index)
                || byte.is_ascii_digit()
                || (b'a'..=b'f').contains(byte)
        })
        && bytes[14] == b'4'
        && matches!(bytes[19], b'8'..=b'9' | b'a'..=b'b')
}

async fn run_secret_tool(
    args: &[String],
    stdin: Option<&str>,
) -> Result<(i32, String, String), String> {
    let flags = gio::SubprocessFlags::STDOUT_PIPE
        | gio::SubprocessFlags::STDERR_PIPE
        | if stdin.is_some() {
            gio::SubprocessFlags::STDIN_PIPE
        } else {
            gio::SubprocessFlags::NONE
        };
    let launcher = gio::SubprocessLauncher::new(flags);
    let mut command = vec![
        "timeout".to_string(),
        "3".to_string(),
        "secret-tool".to_string(),
    ];
    command.extend(args.iter().cloned());
    let argv = command.iter().map(std::ffi::OsStr::new).collect::<Vec<_>>();
    let process = launcher
        .spawn(&argv)
        .map_err(|_| "The desktop keyring tool is unavailable.".to_string())?;
    let (stdout, stderr) = process
        .communicate_utf8_future(stdin.map(str::to_owned))
        .await
        .map_err(|_| "The desktop keyring request failed.".to_string())?;
    Ok((
        process.exit_status(),
        stdout.unwrap_or_default().to_string(),
        stderr.unwrap_or_default().to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(identifiers: &[&str]) -> PinSnapshot {
        PinSnapshot {
            schema: APPLE_PINS_SCHEMA.into(),
            version: 1,
            source: "macos-imcore".into(),
            captured_at: "2026-10-20T16:00:00Z".into(),
            source_modified_at: None,
            pins: identifiers
                .iter()
                .map(|identifier| ApplePin {
                    apple_identifier: (*identifier).into(),
                })
                .collect(),
        }
    }

    fn thread(key: &str, address: &str, is_group: bool) -> Thread {
        Thread {
            key: key.into(),
            name: key.into(),
            recipients: if is_group {
                vec!["+15550100123".into(), "+15550100124".into()]
            } else {
                vec![address.into()]
            },
            last_ts: "2026-10-20".into(),
            is_group,
            ..Thread::default()
        }
    }

    #[test]
    fn accepts_only_bounded_supported_ordered_snapshots() {
        let parsed = PinSnapshot::parse(
            serde_json::to_string(&snapshot(&["+15550100123", "group-token"]))
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(parsed.pins[0].apple_identifier, "+15550100123");
        assert!(PinSnapshot::parse(&vec![b'x'; MAX_IMPORT_BYTES + 1]).is_err());
        assert!(PinSnapshot::parse(b"{\"status\":\"unavailable\"}").is_err());
    }

    #[test]
    fn rejects_ambiguous_matches_and_never_guesses_a_group_by_name() {
        let data = snapshot(&["+15550100123", "group-token"]);
        let threads = vec![
            thread("alex", "+1 (555) 010-0123", false),
            thread("group", "group-token", true),
        ];
        assert_eq!(
            suggested_pin_matches(&data, &threads, None),
            vec![Some("alex".into()), None]
        );
        let mut duplicate = threads.clone();
        duplicate.push(thread("alex-copy", "+15550100123", false));
        assert_eq!(
            suggested_pin_matches(&data, &duplicate, None),
            vec![None, None]
        );
    }

    #[test]
    fn reviewed_order_is_preserved_and_choices_are_unique() {
        let data = snapshot(&["group-token", "+15550100123"]);
        let threads = vec![
            thread("alex", "+15550100123", false),
            thread("group", "", true),
            thread("newer", "+15550100125", false),
        ];
        let state = ImportedPinState::reviewed(
            data.clone(),
            vec![Some("group".into()), Some("alex".into())],
            &threads,
        )
        .unwrap();
        assert_eq!(
            state.ordered_thread_keys(&threads),
            vec!["group", "alex", "newer"]
        );
        assert!(
            ImportedPinState::reviewed(
                data,
                vec![Some("alex".into()), Some("alex".into())],
                &threads,
            )
            .is_err()
        );
    }

    #[test]
    fn preserves_explicit_group_choices_when_apple_reorders() {
        let old = snapshot(&["+15550100123", "group-token"]);
        let old_state = ImportedPinState::reviewed(
            old,
            vec![Some("alex".into()), Some("group".into())],
            &[
                thread("alex", "+15550100123", false),
                thread("group", "", true),
            ],
        )
        .unwrap();
        let reordered = snapshot(&["group-token", "+15550100123"]);
        let threads = vec![
            thread("alex", "+15550100123", false),
            thread("group", "", true),
        ];
        assert_eq!(
            suggested_pin_matches(&reordered, &threads, Some(&old_state)),
            vec![Some("group".into()), Some("alex".into())]
        );
    }

    #[test]
    fn validates_profile_id_before_any_secret_service_operation() {
        assert!(is_valid_phone_profile_id(
            "01234567-89ab-4cde-8fab-0123456789ab"
        ));
        assert!(!is_valid_phone_profile_id("02:00:00:00:00:01"));
        assert!(!is_valid_phone_profile_id(
            "01234567-89ab-1cde-8fab-0123456789ab"
        ));
    }

    #[test]
    fn phone_profile_id_status_field_is_optional_for_older_backends() {
        let old: crate::BlueFerryStatus = serde_json::from_str(r#"{"api_version":2}"#).unwrap();
        assert!(old.phone_profile_id.is_none());
        let current: crate::BlueFerryStatus = serde_json::from_str(
            r#"{"api_version":2,"phone_profile_id":"01234567-89ab-4cde-8fab-0123456789ab"}"#,
        )
        .unwrap();
        assert_eq!(
            current.phone_profile_id.as_deref(),
            Some("01234567-89ab-4cde-8fab-0123456789ab")
        );
    }
}
