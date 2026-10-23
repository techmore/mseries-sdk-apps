use mseries_signal_import::{
    import::{self, Config, Observer, Progress, Stage},
    keyring::SecretTool,
    progress,
};
use std::{
    env,
    io::{IsTerminal, Write},
    path::PathBuf,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Prints the progress line to stderr (rewritten in place on a terminal) and
/// stops cleanly between conversations on Ctrl-C.
struct LineProgress {
    started: Instant,
    last: Instant,
    terminal: bool,
    cancel: Arc<AtomicBool>,
}

impl Observer for LineProgress {
    fn progress(&mut self, p: &Progress) {
        let final_line = matches!(p.stage, Stage::Done | Stage::Cancelled);
        if !final_line && self.last.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last = Instant::now();
        let text = progress::line(p, self.started.elapsed());
        if self.terminal {
            eprint!("\r\x1b[2K{text}");
            if final_line {
                eprintln!();
            }
        } else {
            eprintln!("{text}");
        }
        let _ = std::io::stderr().flush();
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

fn install_ctrl_c(flag: &Arc<AtomicBool>) {
    // SAFETY-free approach: poll SIGINT through a tiny handler that sets the flag.
    static FLAG: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
    let _ = FLAG.set(flag.clone());
    extern "C" fn handler(_: i32) {
        if let Some(flag) = FLAG.get() {
            flag.store(true, Ordering::Relaxed);
        }
    }
    unsafe extern "C" {
        fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    }
    // SAFETY: `handler` only performs an atomic store, which is async-signal-safe.
    unsafe { signal(2, handler) };
}

const USAGE: &str = "usage: mseries-signal-import <import|probe> [options]

  import   copy the Signal Desktop history into Signal Pantheon (safe to repeat)
  probe    show how the Pantheon store is laid out (counts and shapes only)

options:
  --dry-run              read and compare, write nothing
  --signal-dir DIR       Signal Desktop data (default ~/.config/Signal Beta)
  --app-name NAME        keyring application name (default \"Signal Beta\")
  --history-db FILE      Pantheon history (default ~/.local/share/signal-pantheon/message-history.db)
  --accounts-json FILE   signal-cli accounts file (default next to the history database)
  --account-key VALUE    override the account part of the Pantheon store keys
";

fn parse() -> Result<(String, Config), String> {
    let home = PathBuf::from(env::var_os("HOME").ok_or("HOME is not set")?);
    let mut args = env::args().skip(1);
    let command = args.next().ok_or(USAGE)?;
    if command != "import" && command != "probe" {
        return Err(USAGE.into());
    }
    let mut cfg = Config {
        signal_dir: home.join(".config/Signal Beta"),
        app_name: "Signal Beta".into(),
        history_db: home.join(".local/share/signal-pantheon/message-history.db"),
        accounts_json: PathBuf::new(),
        scratch_dir: env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(env::temp_dir),
        dry_run: false,
        check_running: true,
        account_override: None,
    };
    let mut accounts_set = false;
    while let Some(flag) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match flag.as_str() {
            "--dry-run" => cfg.dry_run = true,
            "--signal-dir" => cfg.signal_dir = value("--signal-dir")?.into(),
            "--app-name" => cfg.app_name = value("--app-name")?,
            "--history-db" => cfg.history_db = value("--history-db")?.into(),
            "--accounts-json" => {
                cfg.accounts_json = value("--accounts-json")?.into();
                accounts_set = true;
            }
            "--account-key" => cfg.account_override = Some(value("--account-key")?),
            _ => return Err(format!("unknown option {flag}\n\n{USAGE}")),
        }
    }
    if !accounts_set {
        cfg.accounts_json = cfg.history_db.with_file_name("data").join("accounts.json");
    }
    Ok((command, cfg))
}

fn main() -> ExitCode {
    let (command, cfg) = match parse() {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let result = if command == "probe" {
        import::probe(&cfg, &SecretTool)
    } else {
        let cancel = Arc::new(AtomicBool::new(false));
        install_ctrl_c(&cancel);
        let mut observer = LineProgress {
            started: Instant::now(),
            last: Instant::now() - Duration::from_secs(1),
            terminal: std::io::stderr().is_terminal(),
            cancel,
        };
        import::import_with(&cfg, &SecretTool, &mut observer).map(|report| report.summary())
    };
    match result {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("import stopped: {message}");
            ExitCode::FAILURE
        }
    }
}
