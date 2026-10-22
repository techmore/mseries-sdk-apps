//! The server's contracts as this client knows them: the orchestration protocol version and the
//! RPC tag list, compiled in from `contracts/` by `build.rs` (which also fails the build when the
//! local t3code clone has moved on). `scripts/refresh-contracts.sh` regenerates the vendored copy.

include!(concat!(env!("OUT_DIR"), "/contracts.rs"));

/// Whether the server's contracts define this RPC tag.
pub fn is_known(tag: &str) -> bool {
    METHODS.binary_search(&tag).is_ok()
}

/// Tags the client calls or subscribes to (literal strings after `.call(` / `.subscribe(` in `src/`).
#[cfg(test)]
fn tags_used_in_sources() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") && p.file_name().is_some_and(|n| n != "contracts.rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let mut tags = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for marker in [".call(", ".subscribe("] {
            for (i, _) in text.match_indices(marker) {
                let rest = text[i + marker.len()..].trim_start();
                if let Some(rest) = rest.strip_prefix('"')
                    && let Some(end) = rest.find('"')
                {
                    tags.push((f.file_name().unwrap().to_string_lossy().into_owned(), rest[..end].to_string()));
                }
            }
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendored_contracts_are_sane() {
        assert_eq!(PROTOCOL_VERSION, 2);
        assert!(METHODS.len() > 100, "{} methods", METHODS.len());
        assert!(METHODS.windows(2).all(|w| w[0] < w[1]), "sorted and unique");
        assert!(is_known("server.getConfig") && is_known("orchestration.subscribeThread") && !is_known("server.nope"));
        assert!(!CONTRACTS_COMMIT.is_empty());
    }

    /// Every tag the client sends must exist in the contracts; a rename upstream fails here.
    #[test]
    fn every_tag_the_client_uses_is_in_the_contracts() {
        let used = tags_used_in_sources();
        assert!(used.len() >= 15, "found the call sites: {used:?}");
        let unknown: Vec<_> = used.iter().filter(|(_, t)| !is_known(t)).collect();
        assert!(unknown.is_empty(), "tags not in contracts/METHODS: {unknown:?}");
    }
}
