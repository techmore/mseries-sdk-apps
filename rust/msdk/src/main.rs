//! `msdk check [--json] <file.ui.json>...`, `msdk schema`, `msdk components`, `msdk tokens [--json]`.

use std::process::ExitCode;

use msdk::{Severity, agent, check, gate, schema, templates, tokens, vocabulary};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("components") => print_json(&vocabulary()),
        Some("schema") => print_json(&schema::schema()),
        Some("tokens") if args.get(1).map(String::as_str) == Some("--json") => print_json(&tokens::tokens()),
        Some("tokens") => {
            print!("{}", tokens::css());
            ExitCode::SUCCESS
        }
        Some("check") if args.len() > 1 => run_check(&args[1..]),
        Some("describe") => match args.get(1) {
            None => print_json(&agent::catalogue()),
            Some(n) => match agent::describe(n) {
                Some(v) => print_json(&v),
                None => {
                    eprintln!("nothing called `{n}`; run `msdk describe` for the catalogue");
                    ExitCode::FAILURE
                }
            },
        },
        Some("explain") if args.len() > 1 => match agent::explain(&args[1]) {
            Some(t) => {
                println!("{t}");
                ExitCode::SUCCESS
            }
            None => {
                eprintln!("unknown code `{}`", args[1]);
                ExitCode::FAILURE
            }
        },
        Some("complete") if args.len() > 2 => {
            let doc = match std::fs::read_to_string(&args[1])
                .map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string()))
            {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("{}: {e}", args[1]);
                    return ExitCode::FAILURE;
                }
            };
            for c in agent::complete(&doc, &args[2]) {
                println!("{}\t{}", c.label, c.detail);
            }
            ExitCode::SUCCESS
        }
        Some("gate") if args.len() > 1 => run_gate(&args[1..]),
        Some("templates") => {
            for t in templates::TEMPLATES {
                println!("{:<15} {}\n{:<15} covers: {}", t.name, t.about, "", t.covers);
            }
            ExitCode::SUCCESS
        }
        Some("new") if args.len() > 2 => run_new(&args[1..]),
        _ => {
            eprintln!(
                "usage: msdk check [--json] <file.ui.json>...\n       msdk describe [Name] | explain <code> | complete <file> <json-pointer>\n       msdk gate <file.ui.json> [gates.json]\n       msdk templates | new <template> <dir> [--id ID] [--name NAME]\n       msdk schema | components | tokens [--json]"
            );
            ExitCode::from(2)
        }
    }
}

fn print_json(v: &serde_json::Value) -> ExitCode {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
    ExitCode::SUCCESS
}

fn run_check(args: &[String]) -> ExitCode {
    let json = args.iter().any(|a| a == "--json");
    let mut failed = false;
    let mut report = Vec::new();
    for file in args.iter().filter(|a| *a != "--json") {
        let doc = match std::fs::read_to_string(file)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str(&t).map_err(|e| format!("line {}: {e}", e.line())))
        {
            Ok(v) => v,
            Err(e) => {
                failed = true;
                if json {
                    report.push(
                        serde_json::json!({ "file": file, "severity": "error", "code": "bad-json", "message": e }),
                    );
                } else {
                    println!("{file}: error[bad-json] {e}");
                }
                continue;
            }
        };
        let diags = check(&doc);
        failed |= diags.iter().any(|d| d.severity == Severity::Error);
        for d in &diags {
            if json {
                let mut o = d.to_json();
                o["file"] = file.clone().into();
                report.push(o);
            } else {
                println!("{file}: {d}");
            }
        }
        if !json && !diags.iter().any(|d| d.severity == Severity::Error) {
            println!("{file}: ok ({} warning(s))", diags.len());
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn run_gate(args: &[String]) -> ExitCode {
    let read = |p: &str| {
        std::fs::read_to_string(p)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string()))
    };
    let doc = match read(&args[0]) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{}: {e}", args[0]);
            return ExitCode::FAILURE;
        }
    };
    let gates = match args.get(1).map(|p| read(p)) {
        Some(Ok(g)) => Some(g),
        Some(Err(e)) => {
            eprintln!("{}: {e}", args[1]);
            return ExitCode::FAILURE;
        }
        None => None,
    };
    let results = gate::run(&doc, gates.as_ref());
    println!(
        "{}",
        serde_json::to_string_pretty(&results.iter().map(|r| r.to_json()).collect::<Vec<_>>()).unwrap_or_default()
    );
    if results.iter().all(|r| r.passed) { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn run_new(args: &[String]) -> ExitCode {
    let Some(t) = templates::find(&args[0]) else {
        eprintln!("no template `{}`; run `msdk templates`", args[0]);
        return ExitCode::FAILURE;
    };
    let dir = std::path::Path::new(&args[1]);
    let opt = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let leaf = dir.file_name().and_then(|n| n.to_str()).unwrap_or("MyApp").to_string();
    let camel: String = leaf
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w[..1].to_uppercase() + &w[1..])
        .collect();
    let id = opt("--id")
        .unwrap_or_else(|| format!("io.github.techmore.{}", if camel.is_empty() { "MyApp" } else { &camel }));
    let name = opt("--name").unwrap_or_else(|| leaf.replace(['-', '_'], " "));
    if dir.exists() {
        eprintln!("{} already exists", dir.display());
        return ExitCode::FAILURE;
    }
    let write = |f: &str, body: String| std::fs::write(dir.join(f), body);
    if let Err(e) = std::fs::create_dir_all(dir)
        .and_then(|_| write("app.ui.json", t.render(&id, &name)))
        .and_then(|_| write("app.gates.json", t.gates.to_string()))
        .and_then(|_| write("README.md", t.readme(&name)))
    {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    println!(
        "created {} from `{}` (id {id}); next: cd {} && msdk check app.ui.json && msdk gate app.ui.json app.gates.json",
        dir.display(),
        t.name,
        dir.display()
    );
    ExitCode::SUCCESS
}
