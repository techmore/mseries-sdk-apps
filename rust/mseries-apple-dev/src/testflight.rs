//! The newest builds of an app in App Store Connect: Apple's processing state and the TestFlight
//! state, read after an upload. Read-only (`GET /v1/apps`, `GET /v1/builds`) through the toolkit's
//! own `tools/asc.py` `call()`, so the API key handling and JWT are the toolkit's. `fetch` blocks.

use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

/// Asks App Store Connect and prints one JSON line: `{"app_id", "builds": [...]}` or `{"error"}`.
/// argv: the toolkit's tools folder, the bundle identifier or an `.ipa` to read it from.
const QUERY: &str = r#"
import json, sys
tools, bundle = sys.argv[1], sys.argv[2]
sys.path.insert(0, tools)
try:
    import asc
    if bundle.endswith(".ipa"):
        import plistlib, zipfile
        with zipfile.ZipFile(bundle) as z:
            info = next(n for n in z.namelist() if n.count("/") == 2 and n.endswith(".app/Info.plist"))
            bundle = plistlib.loads(z.read(info))["CFBundleIdentifier"]
    apps = asc.call("GET", "/v1/apps?filter[bundleId]=" + bundle)["data"]
    if not apps:
        sys.exit(f"No App Store Connect app for {bundle} yet.")
    app_id = apps[0]["id"]
    res = asc.call("GET", "/v1/builds?filter[app]=" + app_id
                   + "&sort=-uploadedDate&limit=3&include=preReleaseVersion,buildBetaDetail")
    included = {(i["type"], i["id"]): i.get("attributes", {}) for i in res.get("included", [])}
    def linked(build, rel):
        data = (build.get("relationships", {}).get(rel) or {}).get("data")
        return included.get((data["type"], data["id"]), {}) if data else {}
    builds = []
    for b in res.get("data", []):
        a = b.get("attributes", {})
        builds.append({
            "version": linked(b, "preReleaseVersion").get("version") or "",
            "build": a.get("version") or "",
            "processing": a.get("processingState") or "",
            "beta": linked(b, "buildBetaDetail").get("internalBuildState") or "",
            "uploaded": a.get("uploadedDate") or "",
            "expired": bool(a.get("expired")),
        })
    print(json.dumps({"app_id": app_id, "builds": builds}))
except SystemExit as e:
    print(json.dumps({"error": str(e.code).removeprefix("asc.py: ")}))
except Exception as e:
    print(json.dumps({"error": f"{type(e).__name__}: {e}"}))
"#;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Build {
    /// Marketing version (`1.0`).
    pub version: String,
    /// Build number (`202610240241`).
    pub build: String,
    /// `PROCESSING`, `FAILED`, `INVALID` or `VALID`.
    pub processing: String,
    /// Internal TestFlight state (`IN_BETA_TESTING`, `MISSING_EXPORT_COMPLIANCE`, …).
    pub beta: String,
    /// ISO 8601.
    pub uploaded: String,
    pub expired: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ready,
    /// Apple is still working on it; check again later.
    Waiting,
    Problem,
}

impl Build {
    pub fn title(&self) -> String {
        match (self.version.is_empty(), self.build.is_empty()) {
            (false, false) => format!("{} ({})", self.version, self.build),
            (true, false) => format!("Build {}", self.build),
            _ => "Build".into(),
        }
    }

    pub fn state(&self) -> (Level, &'static str) {
        if self.expired {
            return (Level::Problem, "Expired in TestFlight");
        }
        match self.processing.as_str() {
            "PROCESSING" => return (Level::Waiting, "Apple is processing the build"),
            "FAILED" => return (Level::Problem, "Processing failed; Apple emails the reason"),
            "INVALID" => return (Level::Problem, "Apple marked the build invalid; check the email it sent"),
            _ => {}
        }
        match self.beta.as_str() {
            "IN_BETA_TESTING" => (Level::Ready, "Testing in TestFlight"),
            "READY_FOR_BETA_TESTING" => (Level::Ready, "Ready to test in TestFlight"),
            "PROCESSING" => (Level::Waiting, "TestFlight is processing the build"),
            "IN_EXPORT_COMPLIANCE_REVIEW" => (Level::Waiting, "In export compliance review"),
            "MISSING_EXPORT_COMPLIANCE" => (Level::Problem, "Needs the export compliance answer in App Store Connect"),
            "PROCESSING_EXCEPTION" => (Level::Problem, "TestFlight processing failed"),
            "EXPIRED" => (Level::Problem, "Expired in TestFlight"),
            _ => (Level::Ready, "Processed"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Builds {
    pub app_id: String,
    /// Newest first.
    pub builds: Vec<Build>,
}

impl Builds {
    /// Worth asking again: nothing listed yet (an upload appears after a minute or two) or the
    /// newest build is still being processed.
    pub fn pending(&self) -> bool {
        self.builds.first().is_none_or(|b| b.state().0 == Level::Waiting)
    }

    pub fn testflight_url(&self) -> String {
        format!("https://appstoreconnect.apple.com/apps/{}/testflight/ios", self.app_id)
    }
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    app_id: String,
    builds: Option<Vec<Build>>,
    error: Option<String>,
}

/// The query's JSON line. Errors keep only their first line (asc.py appends the HTTP body).
pub fn parse(output: &str) -> Result<Builds, String> {
    let line = output.lines().rev().find(|l| l.trim_start().starts_with('{')).unwrap_or("");
    let reply: Reply = serde_json::from_str(line).map_err(|_| "App Store Connect gave no answer.".to_string())?;
    match (reply.error, reply.builds) {
        (Some(e), _) => Err(e.lines().next().unwrap_or("App Store Connect failed").trim().to_string()),
        (None, Some(builds)) => Ok(Builds { app_id: reply.app_id, builds }),
        (None, None) => Err("App Store Connect gave no answer.".into()),
    }
}

/// Screenshots and demos: `MSERIES_APPLE_DEV_FIXTURE_TESTFLIGHT=<reply JSON file>`.
pub fn fixture() -> Option<Result<Builds, String>> {
    let path = std::env::var_os("MSERIES_APPLE_DEV_FIXTURE_TESTFLIGHT")?;
    Some(std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|s| parse(&s)))
}

/// What identifies the app: the bundle ID from xtool.yml, else the newest `.ipa` (Flutter).
pub fn app_ref(project: &crate::project::Project) -> Option<String> {
    project.bundle_id.clone().or_else(|| crate::project::latest_ipa(project).map(|p| p.to_string_lossy().into_owned()))
}

/// `app` is a bundle ID or an `.ipa` path (see [`app_ref`]).
pub fn fetch(toolkit: &Path, app: &str, env: &[(String, String)]) -> Result<Builds, String> {
    if let Some(result) = fixture() {
        return result;
    }
    let python = crate::settings::home().join("pymobile3-venv/bin/python");
    if !python.is_file() {
        return Err("Needs ~/pymobile3-venv; Install Toolchain creates it.".into());
    }
    let tools = toolkit.join("tools");
    let out = crate::toolkit::run_with_env(
        &python.to_string_lossy(),
        &["-c", QUERY, &tools.to_string_lossy(), app],
        None,
        env,
        Duration::from_secs(60),
    )
    .ok_or_else(|| "App Store Connect did not answer within a minute.".to_string())?;
    parse(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(processing: &str, beta: &str) -> Build {
        Build {
            version: "1.0".into(),
            build: "202610240241".into(),
            processing: processing.into(),
            beta: beta.into(),
            uploaded: "2026-10-24T02:41:00-07:00".into(),
            expired: false,
        }
    }

    #[test]
    fn parses_builds_and_errors() {
        let ok = parse(
            r#"warning: noise
{"app_id": "6748", "builds": [{"version": "1.0", "build": "7", "processing": "VALID", "beta": "IN_BETA_TESTING", "uploaded": "2026-10-24T02:41:00-07:00", "expired": false}]}"#,
        )
        .unwrap();
        assert_eq!(ok.app_id, "6748");
        assert_eq!(ok.builds[0].title(), "1.0 (7)");
        assert_eq!(ok.builds[0].state(), (Level::Ready, "Testing in TestFlight"));
        assert!(!ok.pending());
        assert_eq!(ok.testflight_url(), "https://appstoreconnect.apple.com/apps/6748/testflight/ios");
        let err = parse(r#"{"error": "GET /v1/apps: HTTP 401\n{\"errors\": []}"}"#).unwrap_err();
        assert_eq!(err, "GET /v1/apps: HTTP 401");
        assert!(parse("Traceback").is_err());
    }

    #[test]
    fn processing_and_empty_lists_are_pending() {
        assert!(Builds { app_id: "1".into(), builds: vec![] }.pending());
        assert!(Builds { app_id: "1".into(), builds: vec![build("PROCESSING", "")] }.pending());
        assert!(Builds { app_id: "1".into(), builds: vec![build("VALID", "PROCESSING")] }.pending());
        assert!(!Builds { app_id: "1".into(), builds: vec![build("VALID", "MISSING_EXPORT_COMPLIANCE")] }.pending());
    }

    #[test]
    fn states_read_plainly() {
        assert_eq!(build("FAILED", "").state().0, Level::Problem);
        assert_eq!(build("VALID", "READY_FOR_BETA_TESTING").state(), (Level::Ready, "Ready to test in TestFlight"));
        assert_eq!(build("VALID", "").state(), (Level::Ready, "Processed"));
        let mut expired = build("VALID", "IN_BETA_TESTING");
        expired.expired = true;
        assert_eq!(expired.state().0, Level::Problem);
    }

    #[test]
    fn query_is_valid_python() {
        let out = std::process::Command::new("python3")
            .args(["-c", "import ast, sys; ast.parse(sys.stdin.read())"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(QUERY.as_bytes())?;
                child.wait()
            });
        if let Ok(status) = out {
            assert!(status.success());
        }
    }
}
