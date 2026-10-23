//! Smart Reply: the hotkey command, the input service and the fixture tools.
//!
//!   mseries-reply trigger [--in SECONDS] [--dry-run]   draft a reply for the focused chat field (bound to the hotkey)
//!   mseries-reply input-service                        hold the draft; Tab inserts it, Esc discards it (systemd user unit)
//!   mseries-reply memory add|list|edit|rm|export|preview   what Smart Reply knows about you (run it with no arguments for help)
//!   mseries-reply init-rules                           write the default rules file unless one exists
//!   mseries-reply probe [TITLE]                        counts only: what the reader sees in a window (no message text)
//!   mseries-reply rules <rules.json> <app-id> [--contact NAME] [--field KIND]
//!   mseries-reply packet <fixture.json> [--rules rules.json]   (fixtures: no window is ever read)
//!   mseries-reply draft  <fixture.json> [--rules rules.json]   (needs an already-loaded local Qwen model)

use mseries_reply::agent::{self, LocalModel};
use mseries_reply::audit::AuditRecord;
use mseries_reply::context::build_packet;
use mseries_reply::fixture::Fixture;
use mseries_reply::marker::ChatMarker;
use mseries_reply::rules::{FieldKind, Query, Rules};
use mseries_reply::{ibus, memory, probe, trigger};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("trigger") => {
            let options = trigger::Options {
                delay: option(&args, "--in")
                    .and_then(|seconds| seconds.parse().ok())
                    .unwrap_or(0.0),
                dry_run: args.iter().any(|arg| arg == "--dry-run"),
            };
            return ExitCode::from(trigger::run(&options));
        }
        Some("input-service") => {
            return match ibus::serve() {
                Ok(()) => ExitCode::SUCCESS,
                Err(message) => {
                    eprintln!("{message}");
                    ExitCode::FAILURE
                }
            };
        }
        _ => {}
    }
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1).cloned())
}

fn field_kind(name: &str) -> Result<FieldKind, String> {
    Ok(match name {
        "chat" => FieldKind::Chat,
        "search" => FieldKind::Search,
        "address" => FieldKind::Address,
        "password" => FieldKind::Password,
        "card" => FieldKind::Card,
        "otp" => FieldKind::Otp,
        "unknown" => FieldKind::Unknown,
        other => return Err(format!("unknown field kind {other}")),
    })
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("init-rules") => {
            let path = Rules::default_path();
            match mseries_reply::rules::ensure_default_file(&path)? {
                true => println!("wrote {}", path.display()),
                false => println!("{} already exists; left unchanged", path.display()),
            }
            Ok(())
        }
        Some("memory") => {
            let store = mseries_ai::AiStore::open_default()?;
            let rules = Rules::load(&Rules::default_path());
            memory::run(&args[1..], &store, &rules, &mut std::io::stdout())
        }
        Some("probe") => probe::run(args.get(1).map(String::as_str)),
        Some("rules") => {
            let (path, app) = (args.get(1), args.get(2));
            let (Some(path), Some(app)) = (path, app) else {
                return Err("usage: mseries-reply rules <rules.json> <app-id> [--contact NAME] [--field KIND]".into());
            };
            let rules = Rules::load(&PathBuf::from(path));
            let contact = option(&args, "--contact");
            let field = field_kind(&option(&args, "--field").unwrap_or_else(|| "chat".into()))?;
            let decision = rules.decide(&Query {
                app_id: app,
                field,
                contact: contact.as_deref(),
            });
            println!("{decision:?}");
            Ok(())
        }
        Some(command @ ("packet" | "draft")) => {
            let path = args.get(1).ok_or("a fixture file is required")?;
            let text = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
            let fixture = Fixture::parse(&text)?;
            let rules = match option(&args, "--rules") {
                Some(path) => Rules::load(&PathBuf::from(path)),
                None => Rules::load(&Rules::default_path()),
            };
            let marked = fixture.marked;
            let marker = move |_: &str| marked.then(ChatMarker::default);
            let packet = match build_packet(&fixture, &rules, &marker) {
                Ok(packet) => packet,
                Err(refusal) => {
                    println!(
                        "{}",
                        AuditRecord::refused(&fixture.app_id, &refusal).to_json()
                    );
                    return Err(format!("no packet: {refusal:?}"));
                }
            };
            if command == "packet" {
                println!("{}", AuditRecord::drafted(&packet, "packet").to_json());
                return Ok(());
            }
            let personal = agent::personal_context(&packet, &rules);
            let started = std::time::Instant::now();
            let result = agent::draft(&packet, &personal, &LocalModel);
            eprintln!("elapsed {:.1}s", started.elapsed().as_secs_f32());
            println!(
                "{}",
                AuditRecord::drafted(
                    &packet,
                    if result.is_ok() {
                        "drafted"
                    } else {
                        "model-error"
                    }
                )
                .with_personal(personal.counts)
                .to_json()
            );
            println!("{}", result?);
            Ok(())
        }
        _ => Err("usage: mseries-reply rules|packet|draft ... (see the file header)".into()),
    }
}
