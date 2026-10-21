//! The operations a UI may ask the updater to perform, and their command-line form.

use std::fmt;

/// Path the root helper executes. Absolute on purpose: the helper runs with a cleaned environment.
pub const UPDATER: &str = "/usr/sbin/mseries-update";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Dev,
}

impl Channel {
    pub fn as_arg(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Dev => "dev",
        }
    }

    pub fn from_arg(name: &str) -> Option<Channel> {
        match name {
            "stable" => Some(Channel::Stable),
            "dev" => Some(Channel::Dev),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Refresh the signed metadata and list available updates.
    Check,
    /// Download and verify an update; it installs at the next restart.
    Stage,
    /// Restore the package set from before the last update.
    Rollback,
    SwitchChannel(Channel),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Rejected(pub String);

impl fmt::Display for Rejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Command {
    /// Arguments after the helper name, as typed on its command line.
    pub fn helper_args(self) -> Vec<&'static str> {
        match self {
            Command::Check => vec!["check"],
            Command::Stage => vec!["stage"],
            Command::Rollback => vec!["rollback"],
            Command::SwitchChannel(channel) => vec!["channel", channel.as_arg()],
        }
    }

    /// Arguments passed to `mseries-update`. Identical today; kept apart so the allowlist
    /// can never be widened by changing what the CLI accepts.
    pub fn updater_args(self) -> Vec<&'static str> {
        self.helper_args()
    }

    /// Strict parser used by the root helper: exact words, exact count, nothing else.
    pub fn parse(args: &[String]) -> Result<Command, Rejected> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        match words.as_slice() {
            ["check"] => Ok(Command::Check),
            ["stage"] => Ok(Command::Stage),
            ["rollback"] => Ok(Command::Rollback),
            ["channel", name] => Channel::from_arg(name)
                .map(Command::SwitchChannel)
                .ok_or_else(|| Rejected(format!("unknown channel '{name}'"))),
            _ => Err(Rejected(format!("not an allowed operation: {}", words.join(" ")))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn every_command_round_trips_through_the_strict_parser() {
        for command in [
            Command::Check,
            Command::Stage,
            Command::Rollback,
            Command::SwitchChannel(Channel::Stable),
            Command::SwitchChannel(Channel::Dev),
        ] {
            let words: Vec<String> = command.helper_args().iter().map(|w| w.to_string()).collect();
            assert_eq!(Command::parse(&words), Ok(command));
        }
    }

    #[test]
    fn nothing_outside_the_allowlist_is_accepted() {
        for bad in [
            vec![],
            vec!["apply", "--now"],
            vec!["init", "--uri", "https://x", "--keyring", "/tmp/k"],
            vec!["config", "auto_stage", "true"],
            vec!["config"],
            vec!["boot"],
            vec!["confirm"],
            vec!["check", "--include-skipped"],
            vec!["stage", "--include-skipped"],
            vec!["channel"],
            vec!["channel", "nightly"],
            vec!["channel", "dev", "extra"],
            vec!["--help"],
            vec!["CHECK"],
            vec![""],
            vec!["status"],
        ] {
            assert!(Command::parse(&args(&bad)).is_err(), "accepted {bad:?}");
        }
    }
}
