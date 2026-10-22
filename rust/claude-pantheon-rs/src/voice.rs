//! Voice input (INTEGRATIONS.md, last item): push-to-talk recording through PipeWire (`pw-record`,
//! 16 kHz mono s16 WAV under `~/.cache/claude-pantheon-rs/voice/`), local transcription with
//! whisper.cpp (`whisper-cli` from the `whisper.cpp` package and a `ggml-*.bin` model), and
//! insertion at the draft's cursor. It never sends: only the owner's Send does, as T3's own voice
//! design keeps it (`docs/internals/voice-input.md` upstream). The draft the recording started in
//! is checked before insertion, so a late transcript never lands in another thread's draft.

use gtk::{gio, glib};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn expand_tilde(p: &str) -> String {
    match (p.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest).to_string_lossy().into_owned(),
        _ => p.to_string(),
    }
}

pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".cache"));
    base.join("claude-pantheon-rs").join("voice")
}

fn in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

/// The whisper.cpp command line, under the names its packages have used.
pub fn whisper_binary() -> Option<PathBuf> {
    ["whisper-cli", "whisper-cpp", "whisper.cpp", "whisper"].iter().find_map(|n| in_path(n))
}

pub fn recorder_binary() -> Option<PathBuf> {
    in_path("pw-record")
}

/// Model search: an explicit path first, then the usual folders, smaller English models first
/// (they transcribe a sentence in well under a second on this Mac's CPU).
pub fn find_model(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = explicit.map(str::trim).filter(|p| !p.is_empty()) {
        let p = PathBuf::from(expand_tilde(p));
        return p.is_file().then_some(p);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let dirs = [
        home.join(".local/share/claude-pantheon-rs/models"),
        home.join(".local/share/mseries-ai/models"),
        home.join(".local/share/whisper.cpp/models"),
        home.join(".cache/whisper.cpp"),
        PathBuf::from("/usr/share/whisper.cpp/models"),
        PathBuf::from("/usr/local/share/whisper.cpp/models"),
    ];
    let mut found: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with("ggml-") && name.ends_with(".bin") && e.path().is_file() {
                    found.push(e.path());
                }
            }
        }
    }
    pick_model(found)
}

/// Preference among found models: base.en, small.en, base, small, tiny.en, tiny, then anything.
pub fn pick_model(mut found: Vec<PathBuf>) -> Option<PathBuf> {
    const ORDER: [&str; 8] = ["base.en", "small.en", "base", "small", "tiny.en", "tiny", "medium.en", "medium"];
    let rank = |p: &Path| {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = name.trim_start_matches("ggml-").trim_end_matches(".bin");
        ORDER.iter().position(|o| *o == stem).unwrap_or(ORDER.len())
    };
    found.sort_by_key(|p| rank(p));
    found.into_iter().next()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Availability {
    pub recorder: Option<PathBuf>,
    pub whisper: Option<PathBuf>,
    pub model: Option<PathBuf>,
}

impl Availability {
    pub fn probe(model: Option<&str>) -> Availability {
        Availability { recorder: recorder_binary(), whisper: whisper_binary(), model: find_model(model) }
    }

    pub fn ready(&self) -> bool {
        self.recorder.is_some() && self.whisper.is_some() && self.model.is_some()
    }

    /// What is missing, in the words of the fix.
    pub fn hint(&self) -> String {
        let mut parts = Vec::new();
        if self.recorder.is_none() {
            parts.push("pw-record (PipeWire) is not installed".to_string());
        }
        if self.whisper.is_none() {
            parts.push("whisper-cli is not installed (sudo apt install whisper.cpp)".to_string());
        }
        if self.model.is_none() {
            parts.push(
                "no ggml-*.bin model found: put one (ggml-base.en.bin from huggingface.co/ggerganov/whisper.cpp) in ~/.local/share/claude-pantheon-rs/models or name it in Settings → Advanced"
                    .to_string(),
            );
        }
        parts.join("; ")
    }
}

/// A running `pw-record`.
pub struct Recorder {
    proc: gio::Subprocess,
    pub path: PathBuf,
    pub started: Instant,
}

pub fn start_recording() -> Result<Recorder, String> {
    let Some(bin) = recorder_binary() else { return Err("pw-record is not installed".into()) };
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp =
        glib::DateTime::now_local().and_then(|d| d.format("%Y%m%d-%H%M%S")).map(|s| s.to_string()).unwrap_or_default();
    let path = dir.join(format!("take-{stamp}.wav"));
    let launcher =
        gio::SubprocessLauncher::new(gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_PIPE);
    let bin = bin.to_string_lossy().into_owned();
    let file = path.to_string_lossy().into_owned();
    let argv = [
        bin.as_str(),
        "--rate",
        "16000",
        "--channels",
        "1",
        "--format",
        "s16",
        "--media-category",
        "Capture",
        "--media-role",
        "Communication",
        file.as_str(),
    ];
    let argv: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    let proc = launcher.spawn(&argv).map_err(|e| e.to_string())?;
    Ok(Recorder { proc, path, started: Instant::now() })
}

impl Recorder {
    pub fn is_alive(&self) -> bool {
        !self.proc.has_exited()
    }

    /// Stops the capture (SIGINT lets pw-record finish the WAV header) and returns the file.
    pub async fn stop(self) -> Result<PathBuf, String> {
        if !self.proc.has_exited() {
            self.proc.send_signal(2);
        }
        let _ = self.proc.wait_future().await;
        let size = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if size <= 44 {
            let _ = std::fs::remove_file(&self.path);
            return Err("nothing was recorded (is a microphone connected and PipeWire running?)".into());
        }
        Ok(self.path)
    }

    pub fn cancel(self) {
        if !self.proc.has_exited() {
            self.proc.force_exit();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Seconds of audio in a 16 kHz mono s16 WAV, from its size.
pub fn wav_seconds(bytes: u64) -> f64 {
    bytes.saturating_sub(44) as f64 / (16_000.0 * 2.0)
}

/// Runs whisper.cpp on the file and returns the text. `language` "auto" lets whisper detect it.
pub async fn transcribe(wav: &Path, model: &Path, language: &str) -> Result<String, String> {
    let Some(bin) = whisper_binary() else { return Err("whisper-cli is not installed".into()) };
    let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE);
    let (bin, model_s, wav_s) =
        (bin.to_string_lossy().into_owned(), model.to_string_lossy().into_owned(), wav.to_string_lossy().into_owned());
    let lang = if language.trim().is_empty() { "auto".to_string() } else { language.trim().to_string() };
    let threads = std::thread::available_parallelism().map(|n| n.get().min(6)).unwrap_or(4).to_string();
    let argv = [bin.as_str(), "-m", &model_s, "-f", &wav_s, "-l", &lang, "-t", &threads, "-nt", "-np"];
    let argv: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    let proc = launcher.spawn(&argv).map_err(|e| e.to_string())?;
    let (out, err) = proc.communicate_utf8_future(None).await.map_err(|e| e.to_string())?;
    if !(proc.has_exited() && proc.exit_status() == 0) {
        let err = err.map(|s| s.to_string()).unwrap_or_default();
        return Err(err.lines().last().unwrap_or("whisper failed").to_string());
    }
    Ok(clean_transcript(&out.map(|s| s.to_string()).unwrap_or_default()))
}

/// whisper's plain output, one paragraph: timestamps (if any), `[BLANK_AUDIO]` and bracketed
/// sound tags dropped, whitespace collapsed.
pub fn clean_transcript(raw: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    for line in raw.lines() {
        let mut line = line.trim();
        // "[00:00:00.000 --> 00:00:02.000]   text" when -nt was not honoured
        if line.starts_with('[')
            && line.contains("-->")
            && let Some(end) = line.find(']')
        {
            line = line[end + 1..].trim();
        }
        for token in line.split_whitespace() {
            let is_tag =
                (token.starts_with('[') && token.ends_with(']')) || (token.starts_with('(') && token.ends_with(')'));
            if is_tag
                && token.len() > 2
                && token[1..token.len() - 1].chars().all(|c| c.is_ascii_uppercase() || c == '_')
            {
                continue;
            }
            if token == "[BLANK_AUDIO]" {
                continue;
            }
            words.push(token.to_string());
        }
    }
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcripts_clean_up_and_models_rank() {
        assert_eq!(
            clean_transcript("[00:00:00.000 --> 00:00:02.000]   Hello  there\n[BLANK_AUDIO]\n [MUSIC] friend.\n"),
            "Hello there friend."
        );
        assert_eq!(clean_transcript("  plain   text\n\n"), "plain text");
        let found = vec![
            PathBuf::from("/m/ggml-medium.bin"),
            PathBuf::from("/m/ggml-tiny.bin"),
            PathBuf::from("/m/ggml-base.en.bin"),
        ];
        assert_eq!(pick_model(found).unwrap(), PathBuf::from("/m/ggml-base.en.bin"));
        assert!(pick_model(Vec::new()).is_none());
        assert!((wav_seconds(44 + 32_000) - 1.0).abs() < 1e-9);
        let a = Availability { recorder: None, whisper: None, model: None };
        assert!(!a.ready() && a.hint().contains("whisper.cpp") && a.hint().contains("pw-record"));
    }
}
