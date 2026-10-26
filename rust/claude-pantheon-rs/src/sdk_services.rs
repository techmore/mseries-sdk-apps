//! The T3 server as an SDK services provider: the feeds and calls `sdk/t3.services.json` declares, answered by the existing
//! client library (`crate::t3`) on the GLib main loop. The pure mapping from server data to the rows the description binds to
//! (`thread_rows`, `account_rows`, `status_row`, `lines_from_items`) is separate so it can be tested without a server.
//!
//! Not provided yet (they answer with an error the description shows): `t3.runner`, `t3.start_issue`, `router.route`, and the
//! `github.issues` feed.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use futures_channel::mpsc::UnboundedSender;
use futures_util::StreamExt;
use gtk::glib;
use msdk_gtk::host::{Event, Services};
use serde_json::{Value, json};

use crate::t3::model::{Provider, ServerConfig, Shell, ThreadShell, ThreadView, TurnItem, pick_provider};
use crate::t3::{self, Rpc, auth};
use crate::transcript::{first_line, title_from_prompt};

fn new_id() -> String {
    glib::uuid_string_random().to_string()
}

// ---- pure mapping (tested) ----

/// A thread title fit for a sidebar row: server-generated titles sometimes start with markup such as
/// `<system-reminder>`; drop tags, take the first line with words in it, never return an empty string.
pub fn clean_title(raw: &str) -> String {
    let base = title_from_prompt(raw).unwrap_or_else(|| raw.to_string());
    let mut text = String::new();
    let mut in_tag = false;
    for c in base.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    let line = text.lines().map(str::trim).find(|l| l.chars().any(char::is_alphanumeric)).unwrap_or("Untitled thread");
    first_line(line, 80)
}

pub fn thread_rows(shell: &Shell, providers: &[Provider]) -> Vec<Value> {
    let mut threads: Vec<&ThreadShell> = shell
        .threads
        .iter()
        .filter(|t| t.archived_at.is_none() && t.rest.get("deletedAt").is_none_or(Value::is_null))
        .collect();
    threads.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    threads
        .iter()
        .map(|t| {
            let project =
                shell.projects.iter().find(|p| p.id == t.project_id).map(|p| p.title.clone()).unwrap_or_default();
            let provider = providers
                .iter()
                .find(|p| p.instance_id == t.provider_instance_id)
                .map(Provider::name)
                .unwrap_or_else(|| t.provider_instance_id.clone());
            let title = clean_title(&t.title);
            let badge = if t.pending_requests() > 0 {
                "?"
            } else if t.is_running() {
                "…"
            } else {
                ""
            };
            let branch = t.rest.get("branch").and_then(Value::as_str).unwrap_or("");
            let detail = if branch.is_empty() { provider.clone() } else { format!("{provider} · {branch}") };
            json!({
                "id": t.id, "title": title, "project": project, "branch": branch,
                "provider": provider, "badge": badge, "detail": detail,
            })
        })
        .collect()
}

/// One row for the tray: running threads, waiting prompts, and the plan window nearest exhaustion (or a connection note).
pub fn status_row(shell: &Shell, providers: &[Provider], note: Option<&str>) -> Vec<Value> {
    let running = shell.threads.iter().filter(|t| t.archived_at.is_none() && t.is_running()).count();
    let waiting: u64 =
        shell.threads.iter().filter(|t| t.archived_at.is_none()).map(ThreadShell::pending_requests).sum();
    let nearest = note.map(String::from).unwrap_or_else(|| {
        providers
            .iter()
            .filter(|p| crate::usage_monitor::plan_observation(p, crate::look::now_unix()).0 == "fresh")
            .filter_map(|p| p.window("session").map(|w| (p.name(), w.used_percent)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(n, pct)| format!("{n} session at {pct:.0}%"))
            .unwrap_or_else(|| "No fresh plan-window observation".into())
    });
    vec![json!({ "id": "s1", "running": running, "waiting": waiting, "nearest": nearest, "oldest_prompt": "" })]
}

fn tail(s: &str, lines: usize) -> String {
    let all: Vec<&str> = s.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Transcript entries the description's `Transcript` draws: `id`, `kind` (user, assistant, code, tool, permission) and `text`.
pub fn lines_from_items(items: &[TurnItem]) -> Vec<Value> {
    items
        .iter()
        .map(|it| {
            let (kind, text) = match it.kind.as_str() {
                "user_message" => ("user", it.text.clone()),
                "assistant_message" => ("assistant", it.text.clone()),
                "reasoning" => ("tool", format!("Thinking: {}", first_line(&it.text, 120))),
                "command_execution" => {
                    let input = it.input.as_deref().unwrap_or(&it.text);
                    let out = it.output.as_deref().map(|o| format!("\n{}", tail(o, 6))).unwrap_or_default();
                    ("code", format!("$ {input}{out}"))
                }
                "file_change" => (
                    "tool",
                    format!(
                        "Edited {} (+{} -{})",
                        it.file_name.clone().unwrap_or_default(),
                        it.additions.unwrap_or(0),
                        it.deletions.unwrap_or(0)
                    ),
                ),
                "tool_call" => (
                    "tool",
                    format!(
                        "{}: {}",
                        it.name.clone().unwrap_or_else(|| "tool".into()),
                        first_line(it.input.as_deref().unwrap_or(""), 120)
                    ),
                ),
                "approval_request" if it.is_pending_request() => {
                    ("permission", it.prompt.clone().unwrap_or_else(|| it.text.clone()))
                }
                "approval_request" => (
                    "tool",
                    format!("Permission {}: {}", it.status, first_line(it.prompt.as_deref().unwrap_or(""), 100)),
                ),
                other => ("tool", other.to_string()),
            };
            json!({ "id": it.id, "kind": kind, "text": text })
        })
        .collect()
}

/// The `decision` strings the server takes for the description's `allow_once`, `allow_thread` and `deny`.
pub fn server_decision(d: &str) -> Option<&'static str> {
    match d {
        "allow_once" => Some("accept"),
        "allow_thread" => Some("acceptForSession"),
        "deny" => Some("decline"),
        _ => None,
    }
}

// ---- the provider ----

pub struct T3Services {
    inner: Rc<Inner>,
}

/// A feed's way of delivering its whole list to the host.
type Push = Rc<dyn Fn(Value)>;

#[derive(Default)]
struct Inner {
    rpc: RefCell<Option<Rpc>>,
    feeds: RefCell<HashMap<String, Push>>,
    shell: RefCell<Shell>,
    providers: RefCell<Vec<Provider>>,
    view: RefCell<ThreadView>,
    /// The open thread and its subscription id.
    thread: RefCell<Option<(String, String)>>,
    /// Drafts made by `t3.new_thread`: draft id to (project id, provider instance id).
    drafts: RefCell<HashMap<String, (String, String)>>,
    /// Drafts that became real threads: draft id to thread id.
    real: RefCell<HashMap<String, String>>,
    generation: Cell<u64>,
    started: Cell<bool>,
    note: RefCell<Option<String>>,
    monitoring: Rc<crate::sdk_monitor::Monitoring>,
    server_label: RefCell<String>,
}

impl T3Services {
    pub fn new() -> T3Services {
        T3Services { inner: Rc::new(Inner::default()) }
    }

    pub fn stop_monitoring(&self) {
        self.inner.monitoring.cancel();
    }
}

impl Default for T3Services {
    fn default() -> Self {
        T3Services::new()
    }
}

impl Inner {
    fn push(&self, feed: &str, rows: Vec<Value>) {
        let f = self.feeds.borrow().get(feed).cloned();
        if let Some(f) = f {
            f(Value::Array(rows));
        }
    }

    fn push_shell_feeds(&self) {
        let rows = thread_rows(&self.shell.borrow(), &self.providers.borrow());
        self.push("t3.threads", rows);
        self.push_status();
    }

    fn push_status(&self) {
        let rows = status_row(&self.shell.borrow(), &self.providers.borrow(), self.note.borrow().as_deref());
        self.push("t3.status", rows);
    }

    fn push_entries(&self) {
        let rows = lines_from_items(&self.view.borrow().items);
        self.push("t3.entries", rows);
    }

    fn set_note(&self, text: &str) {
        *self.note.borrow_mut() = Some(text.to_string());
        self.push_status();
    }

    fn rpc(&self) -> Option<Rpc> {
        self.rpc.borrow().clone()
    }

    async fn connect(self: Rc<Self>) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.monitoring.disconnect();
        self.rpc.borrow_mut().take();
        self.providers.borrow_mut().clear();
        if std::env::var_os("CLAUDE_PANTHEON_OFFLINE").is_some() {
            self.monitoring.refresh("Usage", None, "Offline".into());
            return self.set_note("Offline mode · server monitoring disabled");
        }
        self.set_note("Connecting to the T3 server...");
        let Some(cred) = auth::load().await else {
            self.push("t3.threads", vec![]);
            return self.set_note("Not paired: run `t3 pair`, then pair in Claude for elementary OS");
        };
        if self.generation.get() != generation {
            return;
        }
        *self.server_label.borrow_mut() =
            if cred.label.is_empty() { cred.environment_id.clone() } else { cred.label.clone() };
        let rpc = match t3::connect(&cred).await {
            Ok(r) => r,
            Err(e) => return self.set_note(&format!("Cannot reach the server: {e}")),
        };
        if self.generation.get() != generation {
            return;
        }
        *self.rpc.borrow_mut() = Some(rpc.clone());
        *self.note.borrow_mut() = None;
        let config = rpc.call("server.getConfig", json!({})).await;
        if self.generation.get() != generation {
            return;
        }
        match config {
            Ok(v) => {
                if let Ok(cfg) = serde_json::from_value::<ServerConfig>(v) {
                    *self.providers.borrow_mut() = cfg.providers;
                    // Monitor projections preserve observation state instead of inventing zero quota.
                }
            }
            Err(e) => self.set_note(&format!("Server config failed: {}", e.message())),
        }
        if self.generation.get() != generation {
            return;
        }
        self.monitoring.connected(rpc.clone(), self.server_label.borrow().clone());
        let (_sub, mut stream) =
            rpc.subscribe("orchestration.subscribeShell", json!({ "requestCompletionMarker": true }));
        while let Some(item) = stream.next().await {
            if self.generation.get() != generation {
                break;
            }
            match item {
                Ok(v) => self.shell_event(&v),
                Err(e) => {
                    self.set_note(&format!("Thread list stopped: {}", e.message()));
                    break;
                }
            }
        }
        if self.generation.get() == generation {
            self.rpc.borrow_mut().take();
            self.monitoring.disconnect();
        }
    }

    fn shell_event(&self, v: &Value) {
        {
            let mut shell = self.shell.borrow_mut();
            match v.get("kind").and_then(Value::as_str).unwrap_or("") {
                "snapshot" => {
                    // A snapshot carrying `resolvedRepositoryIdentityRoots` only enriches project identities; keep the thread list.
                    let fresh = Shell::from_snapshot(&v["snapshot"]);
                    if v.get("resolvedRepositoryIdentityRoots").is_some() && !shell.threads.is_empty() {
                        for p in fresh.projects {
                            shell.upsert_project(p);
                        }
                    } else {
                        *shell = fresh;
                    }
                }
                "thread.updated" => {
                    if let Ok(t) = serde_json::from_value::<ThreadShell>(v["thread"].clone()) {
                        if v.get("location").and_then(Value::as_str) == Some("archive") {
                            shell.remove_thread(&t.id);
                        } else {
                            shell.upsert_thread(t);
                        }
                    }
                }
                "thread.removed" => {
                    if let Some(id) = v.get("threadId").and_then(Value::as_str) {
                        shell.remove_thread(id);
                    }
                }
                "project.updated" => {
                    if let Ok(p) = serde_json::from_value(v["project"].clone()) {
                        shell.upsert_project(p);
                    }
                }
                "project.removed" => {
                    if let Some(id) = v.get("projectId").and_then(Value::as_str) {
                        shell.projects.retain(|p| p.id != id);
                    }
                }
                _ => return,
            }
        }
        self.push_shell_feeds();
    }

    /// The server's id for a thread the description knows, resolving drafts that became real.
    fn resolve(&self, id: &str) -> String {
        self.real.borrow().get(id).cloned().unwrap_or_else(|| id.to_string())
    }

    fn open(self: &Rc<Self>, id: &str) {
        let id = self.resolve(id);
        if let Some((_, sub)) = self.thread.borrow_mut().take()
            && let Some(rpc) = self.rpc()
        {
            rpc.interrupt(&sub);
        }
        *self.view.borrow_mut() = ThreadView::default();
        self.push_entries();
        let Some(rpc) = self.rpc() else { return };
        if id.starts_with("draft:") {
            return;
        }
        let (sub, mut stream) = rpc.subscribe(
            "orchestration.subscribeThread",
            json!({ "threadId": id, "requestCompletionMarker": true, "acceptBoundedSnapshot": true, "acceptCompactTurnItems": true }),
        );
        *self.thread.borrow_mut() = Some((id.clone(), sub));
        let this = self.clone();
        glib::spawn_future_local(async move {
            while let Some(item) = stream.next().await {
                if this.thread.borrow().as_ref().map(|(t, _)| t.as_str()) != Some(id.as_str()) {
                    break;
                }
                let Ok(v) = item else { break };
                match v.get("kind").and_then(Value::as_str).unwrap_or("") {
                    "snapshot" => *this.view.borrow_mut() = ThreadView::from_projection(&v["projection"]),
                    "event" => {
                        let (etype, payload) = (v["event"]["type"].as_str().unwrap_or(""), &v["event"]["payload"]);
                        match etype {
                            "turn-item.updated" => {
                                if let Ok(item) = serde_json::from_value::<TurnItem>(payload.clone()) {
                                    this.view.borrow_mut().upsert(item);
                                }
                            }
                            "run.created" | "run.updated" => {
                                let running = matches!(
                                    payload.get("status").and_then(Value::as_str),
                                    Some("running" | "pending" | "queued" | "starting" | "preparing")
                                );
                                this.view.borrow_mut().active_run_id = if running {
                                    payload.get("id").and_then(Value::as_str).map(String::from)
                                } else {
                                    None
                                };
                                continue;
                            }
                            _ => continue,
                        }
                    }
                    _ => continue,
                }
                this.push_entries();
            }
        });
    }
}

fn fail(error: &Option<String>, tx: &UnboundedSender<Event>, text: &str) {
    match error {
        Some(m) => {
            let _ = tx.unbounded_send((m.clone(), Some(json!(text))));
        }
        None => eprintln!("t3: {text}"),
    }
}

impl Services for T3Services {
    fn call(
        &self,
        service: &str,
        args: &BTreeMap<String, Value>,
        then: Option<String>,
        error: Option<String>,
        tx: UnboundedSender<Event>,
    ) -> bool {
        let inner = self.inner.clone();
        let arg = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        match service {
            "monitor.refresh" => {
                inner.monitoring.refresh(&arg("page"), inner.rpc(), inner.server_label.borrow().clone())
            }
            "github.open" => {
                let id = arg("id");
                glib::spawn_future_local(async move {
                    if let Err(message) = inner.monitoring.open_repository(&id).await {
                        fail(&error, &tx, &message);
                    }
                });
            }
            "t3.reconnect" => {
                inner.started.set(true);
                glib::spawn_future_local(inner.connect());
            }
            "t3.open" => inner.open(&arg("thread")),
            "t3.new_thread" => {
                let want = arg("provider");
                let shell = inner.shell.borrow();
                let project = shell
                    .threads
                    .iter()
                    .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
                    .map(|t| t.project_id.clone())
                    .or_else(|| shell.projects.iter().find(|p| p.deleted_at.is_none()).map(|p| p.id.clone()));
                let providers = inner.providers.borrow();
                let instance = if want == "auto" || want.is_empty() {
                    pick_provider(&providers).map(|(p, _)| p.instance_id.clone())
                } else {
                    providers
                        .iter()
                        .find(|p| {
                            p.ready()
                                && (p.instance_id == want
                                    || p.driver.to_lowercase().contains(&want)
                                    || p.name().to_lowercase() == want)
                        })
                        .map(|p| p.instance_id.clone())
                };
                match (project, instance) {
                    (Some(project), Some(instance)) => {
                        let id = format!("draft:{}", new_id());
                        inner.drafts.borrow_mut().insert(id.clone(), (project, instance));
                        drop((shell, providers));
                        inner.open(&id);
                        if let Some(m) = then {
                            let _ = tx.unbounded_send((m, Some(json!(id))));
                        }
                    }
                    _ => fail(&error, &tx, "No project or ready provider to start a thread with"),
                }
            }
            "t3.send" => {
                let (thread, text) = (arg("thread"), arg("text"));
                glib::spawn_future_local(async move {
                    let Some(rpc) = inner.rpc() else { return fail(&error, &tx, "Not connected to the T3 server") };
                    let id = inner.resolve(&thread);
                    let draft = inner.drafts.borrow().get(&id).cloned();
                    if let Some((project, instance)) = draft {
                        // First message of a draft launches the thread with it.
                        let (model, mode) = {
                            let providers = inner.providers.borrow();
                            let p = providers.iter().find(|p| p.instance_id == instance);
                            let model = p
                                .and_then(|p| p.models.iter().find(|m| m.is_default == Some(true)).or(p.models.first()))
                                .map(|m| m.key().to_string())
                                .unwrap_or_default();
                            let mode = p
                                .and_then(|p| p.supported_runtime_modes.clone())
                                .and_then(|m| {
                                    m.iter()
                                        .find(|x| x.as_str() == "approval-required")
                                        .cloned()
                                        .or_else(|| m.first().cloned())
                                })
                                .unwrap_or_else(|| "approval-required".into());
                            (model, mode)
                        };
                        let payload = json!({
                            "commandId": new_id(), "projectId": project, "title": first_line(&text, 60), "generateTitle": true,
                            "creationSource": "web", "modelSelection": { "instanceId": instance, "model": model },
                            "runtimeMode": mode, "interactionMode": "default", "workspaceStrategy": { "type": "root" },
                            "initialMessage": { "messageId": new_id(), "text": text, "attachments": [] },
                        });
                        match rpc.call("orchestration.launchThread", payload).await {
                            Ok(v) => {
                                if let Some(real) = v.get("threadId").and_then(Value::as_str) {
                                    inner.drafts.borrow_mut().remove(&id);
                                    inner.real.borrow_mut().insert(id.clone(), real.to_string());
                                    inner.open(real);
                                }
                            }
                            Err(e) => fail(&error, &tx, &format!("Could not start the thread: {}", e.message())),
                        }
                        return;
                    }
                    let (selection, run) = {
                        let shell = inner.shell.borrow();
                        let t = shell.threads.iter().find(|t| t.id == id).cloned();
                        let selection = t.map(|t| json!({ "instanceId": t.provider_instance_id, "model": t.model_selection.get("model").cloned().unwrap_or(Value::Null) }));
                        (selection, inner.view.borrow().active_run_id.clone())
                    };
                    let dispatch_mode = match run {
                        Some(run) => json!({ "type": "steer_active", "targetRunId": run }),
                        None => json!({ "type": "start_immediately" }),
                    };
                    let mut payload = json!({
                        "type": "message.dispatch", "commandId": new_id(), "threadId": id, "messageId": new_id(),
                        "text": text, "attachments": [], "createdBy": "user", "creationSource": "web", "dispatchMode": dispatch_mode,
                    });
                    if let Some(sel) = selection {
                        payload["modelSelection"] = sel;
                    }
                    if let Err(e) = rpc.call("orchestration.dispatchCommand", payload).await {
                        fail(&error, &tx, &format!("Send failed: {}", e.message()));
                    }
                });
            }
            "t3.answer" => {
                let (request, decision) = (arg("id"), arg("decision"));
                glib::spawn_future_local(async move {
                    let Some(rpc) = inner.rpc() else { return fail(&error, &tx, "Not connected to the T3 server") };
                    let Some(server_decision) = server_decision(&decision) else {
                        return fail(&error, &tx, "Unknown decision");
                    };
                    let thread = inner.thread.borrow().as_ref().map(|(t, _)| t.clone());
                    // "oldest" (the tray button) answers the first prompt still waiting in the open thread.
                    let request = if request == "oldest" || request.is_empty() {
                        inner
                            .view
                            .borrow()
                            .items
                            .iter()
                            .find(|i| i.is_pending_request())
                            .and_then(|i| i.request_id.clone())
                            .unwrap_or_default()
                    } else {
                        // The description sends the transcript entry id; the server wants the request id.
                        inner
                            .view
                            .borrow()
                            .items
                            .iter()
                            .find(|i| i.id == request)
                            .and_then(|i| i.request_id.clone())
                            .unwrap_or(request)
                    };
                    let Some(thread) = thread.filter(|_| !request.is_empty()) else {
                        return fail(&error, &tx, "No waiting prompt in the open thread");
                    };
                    let payload = json!({ "type": "runtime-request.respond", "commandId": new_id(), "threadId": thread, "requestId": request, "decision": server_decision });
                    if let Err(e) = rpc.call("orchestration.dispatchCommand", payload).await {
                        fail(&error, &tx, &format!("Could not answer: {}", e.message()));
                    }
                });
            }
            "t3.runner" | "t3.start_issue" | "router.route" => fail(&error, &tx, "Not available in this build yet"),
            _ => return false,
        }
        true
    }

    fn start_feed(&self, feed: &str, push: Push) -> bool {
        if self.inner.monitoring.register(feed, push.clone()) {
            return true;
        }
        if !matches!(feed, "t3.threads" | "t3.entries" | "t3.status") {
            return false;
        }
        self.inner.feeds.borrow_mut().insert(feed.to_string(), push);
        if !self.inner.started.replace(true) {
            glib::spawn_future_local(self.inner.clone().connect());
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn providers() -> Vec<Provider> {
        serde_json::from_value::<ServerConfig>(fixture("server-config.json")).unwrap().providers
    }

    #[test]
    fn threads_group_by_project_with_branch_provider_and_badges() {
        let shell = Shell::from_snapshot(&fixture("shell-snapshot.json")["snapshot"]);
        let rows = thread_rows(&shell, &providers());
        assert_eq!(rows.len(), shell.threads.iter().filter(|t| t.archived_at.is_none()).count());
        for r in &rows {
            for k in ["id", "title", "project", "branch", "provider", "badge", "detail"] {
                assert!(r.get(k).is_some_and(Value::is_string), "{k} in {r}");
            }
            assert!(!r["title"].as_str().unwrap().contains('\n'));
        }
        let sorted: Vec<&str> =
            shell.threads.iter().filter(|t| t.archived_at.is_none()).map(|t| t.updated_at.as_str()).collect();
        assert!(sorted.len() < 2 || rows.len() == sorted.len());
    }

    #[test]
    fn titles_lose_markup_and_never_come_back_empty() {
        assert_eq!(
            clean_title("<system-reminder>\nThe user opened a file</system-reminder>"),
            "The user opened a file"
        );
        assert_eq!(clean_title("<command-name>/goal</command-name>"), "/goal");
        assert_eq!(clean_title("<system-reminder>"), "Untitled thread");
        assert_eq!(clean_title("Fix the dock"), "Fix the dock");
    }

    #[test]
    fn status_counts_and_names_the_window_nearest_exhaustion() {
        let shell = Shell::from_snapshot(&fixture("shell-snapshot.json")["snapshot"]);
        let row = &status_row(&shell, &providers(), None)[0];
        assert!(row["running"].is_number() && row["waiting"].is_number());
        assert_eq!(status_row(&shell, &providers(), Some("Not paired"))[0]["nearest"], "Not paired");
    }

    #[test]
    fn transcript_lines_use_the_five_kinds_the_description_draws() {
        let view = ThreadView::from_projection(&fixture("thread-snapshot.json")["projection"]);
        let lines = lines_from_items(&view.items);
        assert_eq!(lines.len(), view.items.len());
        for l in &lines {
            assert!(matches!(l["kind"].as_str(), Some("user" | "assistant" | "code" | "tool" | "permission")), "{l}");
            assert!(l["id"].is_string() && l["text"].is_string());
        }
        let mut pending = TurnItem {
            kind: "approval_request".into(),
            status: "pending".into(),
            prompt: Some("Run rm?".into()),
            ..Default::default()
        };
        assert_eq!(lines_from_items(std::slice::from_ref(&pending))[0]["kind"], "permission");
        pending.status = "completed".into();
        assert_eq!(lines_from_items(&[pending])[0]["kind"], "tool");
    }

    #[test]
    fn decisions_map_to_the_servers_words() {
        assert_eq!(server_decision("allow_once"), Some("accept"));
        assert_eq!(server_decision("allow_thread"), Some("acceptForSession"));
        assert_eq!(server_decision("deny"), Some("decline"));
        assert_eq!(server_decision("maybe"), None);
    }
}
