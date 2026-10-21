//! Root side of the Software Updates app, run through `pkexec` under the polkit action
//! `org.techmore.mseries.update` (its `exec.path` is this binary, so polkit authorises exactly this program).
//!
//! It accepts one operation from the closed set in `mseries_update_core::command` and replaces
//! itself with `mseries-update`, so exit codes (75 = another update running, 3 = rollback failed)
//! reach the caller unchanged. It is a binary, not a script: pkexec matches an action by the
//! interpreter for scripts.

use mseries_update_core::command::{Command, UPDATER};
use std::os::unix::process::CommandExt;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match Command::parse(&args) {
        Ok(command) => command,
        Err(rejected) => {
            eprintln!("mseries-update-helper: {rejected}");
            return ExitCode::from(2);
        }
    };
    let error = std::process::Command::new(UPDATER)
        .args(command.updater_args())
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LANG", "C.UTF-8")
        .exec();
    eprintln!("mseries-update-helper: cannot run {UPDATER}: {error}");
    ExitCode::from(70)
}
