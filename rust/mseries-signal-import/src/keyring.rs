//! Login-keyring access through `secret-tool` (libsecret). Secrets travel over a
//! pipe into memory; nothing is written to disk or printed.

use crate::secret::Secret;
use std::process::{Command, Stdio};

pub trait Secrets {
    /// Looks up one item by attributes. `Ok(None)` means no such item.
    fn lookup(&self, attributes: &[(&str, &str)]) -> Result<Option<Secret>, String>;
}

pub struct SecretTool;

impl Secrets for SecretTool {
    fn lookup(&self, attributes: &[(&str, &str)]) -> Result<Option<Secret>, String> {
        let mut command = Command::new("secret-tool");
        command.arg("lookup");
        for (name, value) in attributes {
            command.arg(name).arg(value);
        }
        let output = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .map_err(|e| format!("could not run secret-tool (install libsecret-tools): {e}"))?;
        if output.status.success() && !output.stdout.is_empty() {
            Ok(Some(Secret::new(output.stdout)))
        } else {
            Ok(None)
        }
    }
}
