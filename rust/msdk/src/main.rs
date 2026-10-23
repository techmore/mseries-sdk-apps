//! `msdk check <file.ui.json>...` and `msdk components`.

use std::process::ExitCode;

use msdk::{Severity, check, vocabulary};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("components") => {
            println!("{}", serde_json::to_string_pretty(&vocabulary()).unwrap_or_default());
            ExitCode::SUCCESS
        }
        Some("check") if args.len() > 1 => run_check(&args[1..]),
        _ => {
            eprintln!("usage: msdk check <file.ui.json>...\n       msdk components");
            ExitCode::from(2)
        }
    }
}

fn run_check(files: &[String]) -> ExitCode {
    let mut failed = false;
    for file in files {
        let doc = match std::fs::read_to_string(file)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str(&t).map_err(|e| format!("line {}: {e}", e.line())))
        {
            Ok(v) => v,
            Err(e) => {
                println!("{file}: error[bad-json] {e}");
                failed = true;
                continue;
            }
        };
        let diags = check(&doc);
        for d in &diags {
            println!("{file}: {d}");
        }
        let errors = diags.iter().filter(|d| d.severity == Severity::Error).count();
        if errors > 0 {
            failed = true;
        } else {
            println!("{file}: ok ({} warning(s))", diags.len());
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
