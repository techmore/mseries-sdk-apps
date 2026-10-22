//! `--t3-dump DIR [PAIRING]`: connects to the T3 server, records the answers to the calls the app makes
//! as JSON files in DIR (fixtures for the unit tests), and exits. With PAIRING (a token or pairing
//! URL) it pairs first and stores the credential.

use crate::t3::{self, auth};
use gtk::glib;
use serde_json::{Value, json};
use std::path::PathBuf;

const DEFAULT_SERVER: &str = "http://127.0.0.1:3773";

/// `--t3-call TAG [JSON]`: one unary call with the stored credential, answer printed as JSON.
pub fn call(tag: String, payload: Option<String>) -> glib::ExitCode {
    let payload: Value = payload.map(|p| serde_json::from_str(&p).expect("payload is JSON")).unwrap_or(json!({}));
    glib::MainContext::default().block_on(async move {
        let Some(cred) = auth::load().await else {
            eprintln!("no stored credential; run --t3-dump DIR TOKEN first");
            return glib::ExitCode::FAILURE;
        };
        let rpc = match t3::connect(&cred).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("connect failed: {e}");
                return glib::ExitCode::FAILURE;
            }
        };
        let code = match rpc.call(&tag, payload).await {
            Ok(v) => {
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
                glib::ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{tag}: {e}");
                glib::ExitCode::FAILURE
            }
        };
        rpc.close().await;
        code
    })
}

pub fn dump(dir: Option<String>, pairing: Option<String>) -> glib::ExitCode {
    let dir = PathBuf::from(dir.unwrap_or_else(|| "t3-dump".into()));
    std::fs::create_dir_all(&dir).expect("create dump dir");
    let ctx = glib::MainContext::default();
    ctx.block_on(async move {
        let cred = match pairing {
            Some(p) => {
                let (server, token) = auth::parse_pairing(&p, DEFAULT_SERVER);
                match auth::pair(&server, &token).await {
                    Ok(c) => {
                        let at = auth::store(&c).await;
                        eprintln!("paired with {} ({}), credential stored in the {at}", c.server, c.label);
                        c
                    }
                    Err(e) => {
                        eprintln!("pairing failed: {e}");
                        return glib::ExitCode::FAILURE;
                    }
                }
            }
            None => match auth::load().await {
                Some(c) => c,
                None => {
                    eprintln!("no stored credential; pass a pairing token");
                    return glib::ExitCode::FAILURE;
                }
            },
        };
        let rpc = match t3::connect(&cred).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("connect failed: {e}");
                return glib::ExitCode::FAILURE;
            }
        };
        let save = |name: &str, v: &Value| {
            let path = dir.join(format!("{name}.json"));
            std::fs::write(&path, serde_json::to_vec_pretty(v).unwrap()).unwrap();
            eprintln!("wrote {} ({} bytes)", path.display(), v.to_string().len());
        };
        for (name, tag, payload) in [
            ("server.getConfig", "server.getConfig", json!({})),
            ("projects.list", "projects.list", json!({})),
            ("agentSessions.scan", "agentSessions.scan", json!({})),
            ("server.getHostResources", "server.getHostResources", json!({})),
        ] {
            match rpc.call(tag, payload).await {
                Ok(v) => save(name, &v),
                Err(e) => eprintln!("{tag}: {e}"),
            }
        }
        let today = glib::DateTime::now_local().unwrap();
        let since = today.add_days(-30).unwrap();
        let usage = json!({
            "sinceDay": since.format("%Y-%m-%d").unwrap().to_string(),
            "untilDay": today.format("%Y-%m-%d").unwrap().to_string(),
            "timeZone": today.timezone().identifier().to_string(),
        });
        match rpc.call("server.getUsageSummary", usage).await {
            Ok(v) => save("server.getUsageSummary", &v),
            Err(e) => eprintln!("server.getUsageSummary: {e}"),
        }
        // Shell: snapshot then the synchronized marker.
        use futures_util::StreamExt;
        let (shell_id, mut shell) = rpc.subscribe("orchestration.subscribeShell", json!({"requestCompletionMarker": true}));
        let mut first_thread: Option<String> = None;
        let mut n = 0;
        while let Some(item) = shell.next().await {
            match item {
                Ok(v) => {
                    let kind = v.get("kind").and_then(Value::as_str).unwrap_or("?").to_string();
                    save(&format!("shell.{n}.{kind}"), &v);
                    if kind == "snapshot" && first_thread.is_none() {
                        let s = &v["snapshot"];
                        for key in ["threads", "activeThreads"] {
                            if let Some(t) = s.get(key).and_then(Value::as_array).and_then(|a| a.first()) {
                                first_thread = t.get("id").or_else(|| t.get("thread").and_then(|x| x.get("id"))).and_then(Value::as_str).map(str::to_string);
                            }
                        }
                    }
                    n += 1;
                    if kind == "synchronized" || n > 4 {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("shell: {e}");
                    break;
                }
            }
        }
        rpc.interrupt(&shell_id);
        if let Some(thread_id) = first_thread {
            let (id, mut stream) = rpc.subscribe(
                "orchestration.subscribeThread",
                json!({"threadId": thread_id, "requestCompletionMarker": true, "acceptBoundedSnapshot": true, "acceptCompactTurnItems": true}),
            );
            let mut n = 0;
            while let Some(item) = stream.next().await {
                match item {
                    Ok(v) => {
                        let kind = v.get("kind").and_then(Value::as_str).unwrap_or("?").to_string();
                        save(&format!("thread.{n}.{kind}"), &v);
                        n += 1;
                        if kind == "synchronized" || n > 4 {
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("thread: {e}");
                        break;
                    }
                }
            }
            rpc.interrupt(&id);
        }
        rpc.close().await;
        glib::ExitCode::SUCCESS
    })
}
