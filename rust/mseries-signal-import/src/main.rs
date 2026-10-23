use mseries_signal_import::{
    import::{self, Config},
    keyring::SecretTool,
};
use std::{env, path::PathBuf, process::ExitCode};

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
        import::import(&cfg, &SecretTool).map(|report| report.summary())
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
