//! The real `mseries-update status --json` against this app's parser. The Python updater is run from
//! the repository with its state redirected to a scratch directory, so a change to the JSON that the
//! Rust side cannot read fails here instead of on a customer's machine. No display, no root, no network.

use mseries_update_core::model::{Phase, Summary};
use mseries_updates::backend::{Backend, CliBackend, Failure};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{RwLock, RwLockReadGuard};

/// Tests run on parallel threads. A child forked to run one test's updater inherits every fd open in
/// the process until it execs, including another test's script still open for writing, and exec of a
/// file that has a writer open fails with ETXTBSY ("Text file busy"). Writing a script takes this lock
/// exclusively; running the updater holds it shared, so no fork happens while a write fd is open.
static SPAWN: RwLock<()> = RwLock::new(());

fn spawning() -> RwLockReadGuard<'static, ()> {
    SPAWN.read().unwrap_or_else(|e| e.into_inner())
}

fn updater_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mseries/update/mseries-update.py").canonicalize().expect("mseries-update.py")
}

/// A command that runs the real updater against `state_dir`, like /usr/sbin/mseries-update would.
fn wrapper(dir: &Path, state_dir: &Path) -> PathBuf {
    let script = dir.join("mseries-update");
    let staged = dir.join("mseries-update.tmp");
    let body = format!(
        "#!/bin/sh\nexport MSERIES_UPDATE_STATE='{state}' MSERIES_UPDATE_CONFIG='{dir}/none.conf' \
         MSERIES_UPDATE_SOURCES='{dir}/sources' MSERIES_UPDATE_LOG='{dir}/log' MSERIES_UPDATE_POWER='{dir}/power'\n\
         exec python3 -B '{py}' --quiet \"$@\"\n",
        state = state_dir.display(),
        dir = dir.display(),
        py = updater_script().display(),
    );
    {
        // Write, sync and close under a temp name, then rename into place; the lock keeps every
        // spawn out until the write fd is gone.
        let _writing = SPAWN.write().unwrap_or_else(|e| e.into_inner());
        let mut file = fs::File::create(&staged).unwrap();
        file.write_all(body.as_bytes()).unwrap();
        file.set_permissions(fs::Permissions::from_mode(0o755)).unwrap();
        file.sync_all().unwrap();
        drop(file);
        fs::rename(&staged, &script).unwrap();
    }
    fs::create_dir_all(dir.join("power")).unwrap();
    script
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mseries-updates-contract-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("state")).unwrap();
    dir
}

fn summary(name: &str, state_json: Option<&str>) -> Summary {
    let dir = scratch(name);
    if let Some(json) = state_json {
        fs::write(dir.join("state/state.json"), json).unwrap();
    }
    let backend = CliBackend::with_paths(wrapper(&dir, &dir.join("state")), "/nonexistent-helper");
    let status = { let _run = spawning(); backend.status() }.unwrap_or_else(|f| panic!("{name}: {f:?}"));
    let _ = fs::remove_dir_all(&dir);
    Summary::from_status(&status)
}

#[test]
fn a_machine_that_never_ran_the_updater_is_idle_and_unchecked() {
    let s = summary("fresh", None);
    assert_eq!(s.phase, Phase::NeverChecked);
    assert_eq!(s.channel, "stable");
    assert!(!s.can_roll_back && s.can_switch_channel);
}

#[test]
fn states_written_by_the_updater_are_understood() {
    let available = summary(
        "available",
        Some(r#"{"schema":1,"last_check":{"time":"2026-10-21T20:07:11Z","channel":"stable","updates":{"mseries-update":{"from":"0.1.0-0mseries3","to":"0.1.0-0mseries4"}}}}"#),
    );
    assert!(matches!(available.phase, Phase::Available(ref c) if c.len() == 1 && c[0].to == "0.1.0-0mseries4"));
    assert_eq!(available.last_checked, Some(1_792_613_231));

    let staged = summary(
        "staged",
        Some(r#"{"schema":1,"staged":{"id":"s","created":"2026-10-21T20:08:00Z","channel":"stable","target":{"a":"2"},"from":{"a":"1"},"debs":[]}}"#),
    );
    assert!(matches!(staged.phase, Phase::Staged(ref c) if c.len() == 1));

    let waiting = summary(
        "waiting",
        Some(r#"{"schema":1,"pending":{"id":"p","applied":"2026-10-21T20:00:00Z","unconfirmed_boots":1},
            "last_applied":{"id":"p","from":{"a":"1"},"to":{"a":"2"},"applied":"2026-10-21T20:00:00Z","confirmed":null,"rolled_back":null}}"#),
    );
    assert!(matches!(waiting.phase, Phase::AwaitingConfirmation(_)));
    assert!(waiting.can_roll_back && !waiting.can_switch_channel);

    let recovering = summary("recovering", Some(r#"{"schema":1,"journal":{"id":"j","kind":"apply","restore":{"a":"1"},"target":{"a":"2"}}}"#));
    assert_eq!(recovering.phase, Phase::Recovering);
}

#[test]
fn a_missing_updater_is_reported_as_not_installed() {
    let backend = CliBackend::with_paths("/nonexistent/mseries-update", "/nonexistent-helper");
    let status = { let _run = spawning(); backend.status() };
    assert_eq!(status, Err(Failure::NotInstalled));
}

#[test]
fn a_state_file_from_a_newer_updater_is_an_error_not_a_guess() {
    let dir = scratch("newer");
    fs::write(dir.join("state/state.json"), r#"{"schema":2}"#).unwrap();
    let backend = CliBackend::with_paths(wrapper(&dir, &dir.join("state")), "/nonexistent-helper");
    let err = { let _run = spawning(); backend.status() }.unwrap_err();
    let _ = fs::remove_dir_all(&dir);
    assert!(matches!(err, Failure::Failed(ref why) if why.contains("newer mseries-update")), "{err:?}");
}
