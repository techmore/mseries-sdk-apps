use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_ENTER_SENDS: bool = true;

fn path() -> PathBuf {
    gtk::glib::user_config_dir()
        .join("mseries")
        .join("messages.json")
}

pub fn load_enter_sends() -> bool {
    load_enter_sends_from(&path())
}

fn load_enter_sends_from(path: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
        return DEFAULT_ENTER_SENDS;
    };
    let Ok(Value::Object(settings)) = serde_json::from_str(&contents) else {
        return DEFAULT_ENTER_SENDS;
    };
    settings
        .get("enter_sends")
        .and_then(Value::as_bool)
        .unwrap_or(DEFAULT_ENTER_SENDS)
}

pub fn save_enter_sends(value: bool) -> io::Result<()> {
    save_enter_sends_to(&path(), value)
}

fn save_enter_sends_to(path: &Path, value: bool) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "preference path has no parent")
    })?;
    fs::create_dir_all(parent)?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let (temporary, mut file) = (0..32)
        .find_map(|attempt| {
            let temporary = parent.join(format!(
                ".messages-{}-{nonce}-{attempt}.tmp",
                std::process::id()
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
            {
                Ok(file) => Some(Ok((temporary, file))),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .unwrap_or_else(|| {
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not allocate a temporary preference file",
            ))
        })?;

    let result = (|| {
        let mut contents = serde_json::to_vec_pretty(&json!({ "enter_sends": value }))?;
        contents.push(b'\n');
        file.write_all(&contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn test_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mseries-messages-preferences-{}-{nonce}/messages.json",
            std::process::id()
        ))
    }

    #[test]
    fn missing_or_invalid_value_uses_the_qualified_python_default() {
        let path = test_path();
        assert!(load_enter_sends_from(&path));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r#"{"enter_sends":"yes"}"#).unwrap();
        assert!(load_enter_sends_from(&path));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn preference_round_trips_with_owner_only_file_permissions() {
        let path = test_path();
        save_enter_sends_to(&path, false).unwrap();
        assert!(!load_enter_sends_from(&path));
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        save_enter_sends_to(&path, true).unwrap();
        assert!(load_enter_sends_from(&path));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
