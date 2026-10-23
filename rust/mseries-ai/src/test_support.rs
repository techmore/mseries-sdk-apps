//! A throwaway owner-only database directory for unit tests.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct TestDatabase {
    pub directory: PathBuf,
    pub path: PathBuf,
}

impl TestDatabase {
    pub fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory =
            env::temp_dir().join(format!("mseries-ai-test-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&directory).expect("create isolated test directory");
        let path = directory.join("assistant.sqlite3");
        Self { directory, path }
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
