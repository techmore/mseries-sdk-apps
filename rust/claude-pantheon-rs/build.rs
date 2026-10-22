//! Compiles the vendored T3 contracts (`contracts/PROTOCOL_VERSION`, `contracts/METHODS`) into
//! `t3::contracts`, and when the sparse clone of pingdotgg/t3code is on this machine
//! (`T3_CONTRACTS_REPO` or `~/claude-work/repos/t3code`) compares the vendored copy with it: drift
//! fails the build, naming `scripts/refresh-contracts.sh` (set `T3_CONTRACTS_DRIFT=warn` to only warn).
//! CI has no clone, so there the vendored copy is the contract.

use std::path::{Path, PathBuf};

fn vendored(dir: &Path) -> (u32, Vec<String>, String) {
    let version = std::fs::read_to_string(dir.join("PROTOCOL_VERSION")).expect("contracts/PROTOCOL_VERSION");
    let version: u32 = version.trim().parse().expect("a protocol version number");
    let text = std::fs::read_to_string(dir.join("METHODS")).expect("contracts/METHODS");
    let commit = text
        .lines()
        .find_map(|l| l.strip_prefix("# from commit "))
        .and_then(|l| l.split_whitespace().next())
        .unwrap_or("unknown")
        .to_string();
    let methods: Vec<String> =
        text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect();
    (version, methods, commit)
}

/// The same extraction the refresh script does, so the build can notice an updated clone.
fn upstream(src: &Path) -> Option<(u32, Vec<String>)> {
    let env = std::fs::read_to_string(src.join("environment.ts")).ok()?;
    let version = env
        .lines()
        .find_map(|l| l.trim().strip_prefix("export const ORCHESTRATION_PROTOCOL_VERSION = "))
        .and_then(|v| v.trim_end_matches(';').trim().parse::<u32>().ok())?;
    let mut methods = Vec::new();
    for (file, name) in [("rpc.ts", "WS_METHODS"), ("orchestrationV2.ts", "ORCHESTRATION_V2_WS_METHODS")] {
        let text = std::fs::read_to_string(src.join(file)).ok()?;
        let start = text.find(&format!("export const {name} = {{"))?;
        let body = &text[start..];
        let end = body.find("\n}")?;
        for line in body[..end].lines() {
            if let Some((_, value)) = line.split_once(':')
                && let Some(q) = value.find('"')
                && let Some(e) = value[q + 1..].find('"')
            {
                methods.push(value[q + 1..q + 1 + e].to_string());
            }
        }
    }
    methods.sort();
    methods.dedup();
    Some((version, methods))
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dir = manifest.join("contracts");
    println!("cargo:rerun-if-changed=contracts/PROTOCOL_VERSION");
    println!("cargo:rerun-if-changed=contracts/METHODS");
    println!("cargo:rerun-if-env-changed=T3_CONTRACTS_REPO");
    println!("cargo:rerun-if-env-changed=T3_CONTRACTS_DRIFT");
    let (version, methods, commit) = vendored(&dir);

    let repo = std::env::var_os("T3_CONTRACTS_REPO")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("claude-work/repos/t3code")));
    if let Some(src) = repo.map(|r| r.join("packages/contracts/src")).filter(|p| p.is_dir()) {
        for f in ["environment.ts", "rpc.ts", "orchestrationV2.ts"] {
            println!("cargo:rerun-if-changed={}", src.join(f).display());
        }
        if let Some((up_version, up_methods)) = upstream(&src) {
            let mut drift = Vec::new();
            if up_version != version {
                drift.push(format!("protocol version {version} vendored, {up_version} upstream"));
            }
            let added: Vec<&String> = up_methods.iter().filter(|m| !methods.contains(m)).collect();
            let removed: Vec<&String> = methods.iter().filter(|m| !up_methods.contains(m)).collect();
            if !added.is_empty() {
                drift.push(format!(
                    "new upstream methods: {}",
                    added.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                ));
            }
            if !removed.is_empty() {
                drift.push(format!(
                    "methods gone upstream: {}",
                    removed.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                ));
            }
            if !drift.is_empty() {
                let msg = format!(
                    "T3 contracts drift between contracts/ and {}: {}. Run scripts/refresh-contracts.sh (and --record for fixtures).",
                    src.display(),
                    drift.join("; ")
                );
                if std::env::var("T3_CONTRACTS_DRIFT").as_deref() == Ok("warn") {
                    println!("cargo:warning={msg}");
                } else {
                    panic!("{msg}");
                }
            }
        }
    }

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("contracts.rs");
    let list = methods.iter().map(|m| format!("    {m:?},\n")).collect::<String>();
    std::fs::write(
        &out,
        format!(
            "/// `ORCHESTRATION_PROTOCOL_VERSION` from the vendored contracts.\npub const PROTOCOL_VERSION: u32 = {version};\n\
             /// The t3code commit the contracts were taken from.\npub const CONTRACTS_COMMIT: &str = {commit:?};\n\
             /// Every RPC tag the server's contracts define (sorted).\npub const METHODS: &[&str] = &[\n{list}];\n"
        ),
    )
    .unwrap();
}
