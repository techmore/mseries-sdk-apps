//! The greeter's `Logic` for the SDK renderer: wires the pure login state machine ([`crate::state`]) to the greetd socket
//! through a worker thread, and mirrors the machine's phase into the description's state keys.

use std::path::PathBuf;
use std::sync::mpsc;

use msdk::logic::{Effect, Logic, Store};
use serde_json::{Value, json};

use crate::greetd::{Request, Response};
use crate::state::{Cmd, Msg, Phase, State, update};

/// What the worker thread does with the socket.
pub enum Work {
    Send(Request),
    /// Cancel the session; the reply is not shown.
    Cancel,
}

pub struct GreeterLogic {
    state: State,
    work: mpsc::Sender<Work>,
    last_user_file: PathBuf,
    last_user: Option<String>,
}

fn effect_power(verb: &str) -> Effect {
    Effect::Call {
        service: format!("power.{verb}"),
        args: Default::default(),
        then: None,
        error: None,
    }
}

impl GreeterLogic {
    pub fn new(
        session_cmd: Vec<String>,
        last_user: Option<String>,
        last_user_file: PathBuf,
        work: mpsc::Sender<Work>,
    ) -> GreeterLogic {
        GreeterLogic {
            state: State::new(None, session_cmd),
            work,
            last_user_file,
            last_user,
        }
    }

    fn run(&mut self, msg: Msg) {
        match update(&mut self.state, msg) {
            Cmd::Send(r) => {
                let _ = self.work.send(Work::Send(r));
            }
            Cmd::Cancel => {
                let _ = self.work.send(Work::Cancel);
            }
            Cmd::None => {}
        }
    }

    /// Mirrors the machine's phase into the state keys the description binds to.
    fn sync(&self, store: &mut Store) {
        let s = &self.state;
        let prompt = matches!(s.phase, Phase::Prompt { .. });
        store.insert("needs_user".into(), json!(s.phase == Phase::User));
        store.insert("needs_answer".into(), json!(prompt));
        store.insert(
            "prompt".into(),
            json!(s.prompt.trim().trim_end_matches(':').trim()),
        );
        store.insert("has_notice".into(), json!(s.notice.is_some()));
        store.insert(
            "notice".into(),
            json!(
                s.notice
                    .as_ref()
                    .map(|n| n.text.clone())
                    .unwrap_or_default()
            ),
        );
        if prompt {
            store.insert("username".into(), json!(s.username));
        }
    }

    fn text(store: &Store, key: &str) -> String {
        store
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }
}

impl Logic for GreeterLogic {
    fn update(&mut self, store: &mut Store, msg: &str, payload: Option<Value>) -> Vec<Effect> {
        let mut effects = Vec::new();
        match msg {
            // Start-up: a remembered user goes straight to the password step.
            "_start" => {
                if let Some(u) = self.last_user.take() {
                    store.insert("username".into(), json!(u.clone()));
                    self.run(Msg::SubmitUser(u));
                }
            }
            "SubmitUser" => {
                let name = Self::text(store, "username");
                self.run(Msg::SubmitUser(name));
            }
            "SubmitAnswer" => {
                let answer = Self::text(store, "answer");
                store.insert("answer".into(), json!(""));
                self.run(Msg::SubmitAnswer(Some(answer)));
            }
            "ChangeUser" => {
                self.run(Msg::ChangeUser);
                store.insert("answer".into(), json!(""));
            }
            "Shutdown" => effects.push(effect_power("poweroff")),
            "Restart" => effects.push(effect_power("reboot")),
            "_greetd" => {
                if let Some(reply) =
                    payload.and_then(|p| serde_json::from_value::<Response>(p).ok())
                {
                    let finishing = self.state.phase == Phase::Done && reply == Response::Success;
                    self.run(Msg::Reply(reply));
                    if finishing {
                        if let Some(dir) = self.last_user_file.parent() {
                            let _ = std::fs::create_dir_all(dir);
                        }
                        let _ =
                            std::fs::write(&self.last_user_file, self.state.username.as_bytes());
                        effects.push(Effect::Quit);
                    }
                }
            }
            "_lost" => self.run(Msg::ConnectionLost(
                payload
                    .and_then(|p| p.as_str().map(String::from))
                    .unwrap_or_default(),
            )),
            _ => {}
        }
        self.sync(store);
        effects
    }
}
