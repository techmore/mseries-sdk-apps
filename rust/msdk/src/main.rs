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
            load_services(Some(&args[1]), &args);
            let doc = match load(&args[1]) {
                Ok((d, _)) => d,
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
        Some("fmt") if args.len() > 1 => run_fmt(&args[1..]),
        Some("render") if args.len() > 1 => run_render(&args[1..]),
        Some("text") if args.len() > 1 => convert(&args[1], true),
        Some("json") if args.len() > 1 => convert(&args[1], false),
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
                "usage: msdk check [--json] <file.ui.json>...\n       msdk describe [Name] | explain <code> | complete <file> <json-pointer>\n       msdk gate <file.ui.json> [gates.json]\n       msdk render <file> [--surface NAME] [--state JSON] [--json]\n       msdk fmt [--check|--stdout] <file.msdk>... | text <file.ui.json> | json <file.msdk>\n       msdk templates | new <template> <dir> [--text] [--id ID] [--name NAME]\n       msdk schema | components | tokens [--json]"
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
    for file in files_of(args) {
        load_services(Some(file), args);
        let (doc, parsed) = match load(file) {
            Ok(v) => v,
            Err(e) => {
                failed = true;
                if json {
                    report
                        .push(serde_json::json!({ "file": file, "severity": "error", "code": "syntax", "message": e }));
                } else {
                    println!("{file}: error[syntax] {e}");
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
                o["line"] = parsed
                    .as_ref()
                    .and_then(|p| p.line_for(&d.path))
                    .map(|l| l.into())
                    .unwrap_or(serde_json::Value::Null);
                report.push(o);
            } else {
                match parsed.as_ref().and_then(|p| p.line_for(&d.path)) {
                    Some(l) => println!("{file}:{l}: {d}"),
                    None => println!("{file}: {d}"),
                }
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
    load_services(args.first().map(String::as_str), args);
    let read = |p: &str| {
        std::fs::read_to_string(p)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string()))
    };
    let doc = match load(&args[0]) {
        Ok((d, _)) => d,
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
    let as_text = args.iter().any(|a| a == "--text");
    let rendered = t.render(&id, &name);
    let (ui_file, ui_body) = if as_text {
        match serde_json::from_str::<serde_json::Value>(&rendered)
            .map_err(|e| e.to_string())
            .and_then(|d| msdk::text::print(&d))
        {
            Ok(text) => ("app.msdk", text),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        ("app.ui.json", rendered)
    };
    let write = |f: &str, body: String| std::fs::write(dir.join(f), body);
    if let Err(e) = std::fs::create_dir_all(dir)
        .and_then(|_| write(ui_file, ui_body.clone()))
        .and_then(|_| write("app.gates.json", t.gates.to_string()))
        .and_then(|_| write("README.md", t.readme(&name).replace("app.ui.json", ui_file)))
    {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    println!(
        "created {} from `{}` (id {id}); next: cd {} && msdk check {ui_file} && msdk gate {ui_file} app.gates.json",
        dir.display(),
        t.name,
        dir.display()
    );
    ExitCode::SUCCESS
}

/// Loads a description from `.msdk` text or JSON.
fn load(path: &str) -> Result<(serde_json::Value, Option<msdk::text::Parsed>), String> {
    msdk::text::load(std::path::Path::new(path))
}

/// `msdk text file.ui.json` / `msdk json file.msdk`: print the other spelling.
fn convert(file: &str, to_text: bool) -> ExitCode {
    let doc = match load(file) {
        Ok((d, _)) => d,
        Err(e) => {
            eprintln!("{file}: {e}");
            return ExitCode::FAILURE;
        }
    };
    if to_text {
        match msdk::text::print(&doc) {
            Ok(t) => print!("{t}"),
            Err(e) => {
                eprintln!("{file}: cannot print as text: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&doc).unwrap_or_default());
    }
    ExitCode::SUCCESS
}

/// `msdk fmt [--check] [--stdout] file.msdk...`: rewrite text files in the canonical layout.
fn run_fmt(args: &[String]) -> ExitCode {
    let (check_only, stdout) = (args.iter().any(|a| a == "--check"), args.iter().any(|a| a == "--stdout"));
    let mut bad = false;
    for file in args.iter().filter(|a| !a.starts_with("--")) {
        let original = match std::fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{file}: {e}");
                bad = true;
                continue;
            }
        };
        let formatted =
            match msdk::text::parse(&original).map_err(|e| e.to_string()).and_then(|p| msdk::text::print(&p.doc)) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("{file}: {e}");
                    bad = true;
                    continue;
                }
            };
        if stdout {
            print!("{formatted}");
        } else if check_only {
            if formatted != original {
                println!("{file}: would reformat");
                bad = true;
            }
        } else if formatted != original {
            if let Err(e) = std::fs::write(file, &formatted) {
                eprintln!("{file}: {e}");
                bad = true;
            } else {
                println!("{file}: formatted");
            }
        }
    }
    if bad { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// Loads plug-in service manifests: every `*.services.json` next to `file`, plus any `--services path` flags.
fn load_services(file: Option<&str>, args: &[String]) {
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    if let Some(dir) = file.and_then(|f| std::path::Path::new(f).parent()) {
        let dir = if dir.as_os_str().is_empty() { std::path::Path::new(".") } else { dir };
        if let Ok(rd) = std::fs::read_dir(dir) {
            paths.extend(rd.flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().ends_with(".services.json")));
        }
    }
    for (i, a) in args.iter().enumerate() {
        if a == "--services"
            && let Some(p) = args.get(i + 1)
        {
            paths.push(p.into());
        }
    }
    for p in paths {
        match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
            .and_then(|v| msdk::vocab::load_manifest(&v))
        {
            Ok(_) => {}
            Err(e) => eprintln!("{}: {e}", p.display()),
        }
    }
}

/// The positional arguments with flags (and the value after `--services`) removed.
fn files_of(args: &[String]) -> Vec<&String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if a == "--services" {
            skip = true;
        } else if !a.starts_with("--") {
            out.push(a);
        }
    }
    out
}

/// `msdk render app.msdk [--surface main] [--state '{"key":value}'] [--json]`: what the app shows, headless.
fn run_render(args: &[String]) -> ExitCode {
    let file = args[0].as_str();
    load_services(Some(file), args);
    let doc = match load(file) {
        Ok((d, _)) => d,
        Err(e) => {
            eprintln!("{file}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let opt = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let surface = opt("--surface").unwrap_or_else(|| "main".into());
    let overrides: Option<msdk::logic::Store> =
        match opt("--state").map(|j| serde_json::from_str::<msdk::logic::Store>(&j)) {
            Some(Ok(m)) => Some(m),
            Some(Err(e)) => {
                eprintln!("--state must be a JSON object: {e}");
                return ExitCode::FAILURE;
            }
            None => None,
        };
    match msdk::inspect::render(&doc, &surface, overrides.as_ref()) {
        Ok(tree) if args.iter().any(|a| a == "--json") => print_json(&tree),
        Ok(tree) => {
            print!("{}", msdk::inspect::outline(&tree));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
