//! Agent broker core (design: `mseries/SDK-AGENT-SURFACE.md` section 3).
//!
//! Decides, for one agent call against one app's SDK manifest, whether to run it, ask the owner first, or refuse. It never
//! grants anything by itself: grants come only from [`Broker::answer`], which the owner's consent prompt calls. Model output,
//! tool annotations and app text cannot reach it.
//!
//! Rules enforced here:
//! * a call must be one of the manifest's intents (or an entity query) with arguments that pass `msdk::intents::validate_args`;
//!   a bad call is refused *before* any prompt, so a hostile argument is never shown as a request;
//! * a grant is `(agent, app, scope)` bound to the manifest hash: an app update that changes the manifest asks again;
//! * `read` and `reversible` scopes can be granted for a time or for good; `external` and `privileged` are approved per action
//!   (a fingerprint of the arguments and of the state the confirmation shows, so changing the draft after approval invalidates
//!   it) unless the owner explicitly widens the scope;
//! * a denial is remembered for a while so an agent cannot nag; revocation is immediate;
//! * the audit trail records agent, app, target and outcome, never arguments or content.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use msdk::intents::{canonical, sha256_hex, validate_args};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// How long a denial is remembered.
pub const DENIAL_SECS: u64 = 24 * 3600;
/// How long a per-action approval stays valid after the owner gives it.
pub const APPROVAL_SECS: u64 = 120;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Risk {
    Read,
    Reversible,
    External,
    Privileged,
}

impl Risk {
    fn parse(s: &str) -> Option<Risk> {
        Some(match s {
            "read" => Risk::Read,
            "reversible" => Risk::Reversible,
            "external" => Risk::External,
            "privileged" => Risk::Privileged,
            _ => return None,
        })
    }

    fn per_action(self) -> bool {
        self >= Risk::External
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target<'a> {
    Intent(&'a str),
    /// List the exposed fields of an entity.
    Query(&'a str),
}

#[derive(Clone, Debug)]
pub struct Request<'a> {
    pub agent: &'a str,
    pub app_id: &'a str,
    pub target: Target<'a>,
    pub args: Value,
    /// Current values of the state keys the intent `show`s, read from the app by the caller (not by the agent).
    pub shown: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    UnknownApp,
    UnknownTarget,
    BadArguments(String),
    /// The owner denied this recently.
    Denied,
}

/// What the owner is asked. Everything in it comes from the manifest and the caller, never from the agent's own words.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prompt {
    pub id: u64,
    pub agent: String,
    pub app_id: String,
    pub app_name: String,
    pub scope: String,
    pub scope_about: String,
    pub risk: Risk,
    pub target: String,
    pub about: String,
    /// The argument and the displayed state, for the concrete action.
    pub arguments: Value,
    pub shown: Value,
    /// Choices that make sense for this risk.
    pub choices: Vec<Answer>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// Send this payload to the app's message (`None` for a parameterless intent).
    Run(Option<Value>),
    Ask(Box<Prompt>),
    Refuse(Refusal),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Answer {
    Deny,
    /// This one action (external and privileged), or one call (read and reversible).
    Once,
    /// For this many seconds. read and reversible only.
    For(u64),
    /// Until revoked. read and reversible only.
    Always,
    /// Until revoked, without asking per action. external and privileged only, and the owner must choose it deliberately.
    Widen,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Invalid(pub &'static str);

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct Grant {
    agent: String,
    app_id: String,
    scope: String,
    manifest: String,
    /// `None` = until revoked.
    expires: Option<u64>,
    widened: bool,
    /// `Some(n)`: n more calls (an "allow once" for a read or reversible scope).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    uses_left: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct Denial {
    agent: String,
    app_id: String,
    scope: String,
    until: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    version: u32,
    grants: Vec<Grant>,
    denials: Vec<Denial>,
}

#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
pub struct AuditEntry {
    pub time: u64,
    pub agent: String,
    pub app_id: String,
    pub target: String,
    pub outcome: &'static str,
}

struct Pending {
    prompt: Prompt,
    fingerprint: String,
    manifest: String,
}

struct Approval {
    agent: String,
    app_id: String,
    fingerprint: String,
    manifest: String,
    until: u64,
}

pub struct Broker {
    manifests: BTreeMap<String, Value>,
    saved: Saved,
    pending: BTreeMap<u64, Pending>,
    approvals: Vec<Approval>,
    next_prompt: u64,
    audit: Vec<AuditEntry>,
    path: Option<PathBuf>,
}

fn owner_only(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o077 == 0)
}

impl Broker {
    /// A broker that keeps grants in memory only.
    pub fn in_memory() -> Self {
        Broker {
            manifests: BTreeMap::new(),
            saved: Saved {
                version: 1,
                ..Saved::default()
            },
            pending: BTreeMap::new(),
            approvals: vec![],
            next_prompt: 1,
            audit: vec![],
            path: None,
        }
    }

    /// Loads grants from `path`. A missing, malformed, wrong-version or group/other-accessible file means **no grants**: the safe
    /// state is to ask again.
    pub fn open(path: &Path) -> Self {
        let mut b = Self::in_memory();
        b.path = Some(path.to_path_buf());
        if owner_only(path)
            && let Ok(text) = std::fs::read_to_string(path)
            && let Ok(saved) = serde_json::from_str::<Saved>(&text)
            && saved.version == 1
        {
            b.saved = saved;
        }
        b
    }

    fn save(&self) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(&self.saved).unwrap_or_default(),
        )?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        std::fs::rename(tmp, path)
    }

    /// Registers (or replaces) an app's manifest, as generated by `msdk agent-manifest`.
    pub fn register(&mut self, manifest: Value) -> Result<(), &'static str> {
        let id = manifest["app"]["id"]
            .as_str()
            .ok_or("manifest has no app id")?
            .to_string();
        if manifest["hash"].as_str().is_none_or(|h| h.len() != 64) {
            return Err("manifest has no hash");
        }
        self.manifests.insert(id, manifest);
        Ok(())
    }

    pub fn audit(&self) -> &[AuditEntry] {
        &self.audit
    }

    fn log(&mut self, now: u64, r: &Request, outcome: &'static str) {
        let target = match r.target {
            Target::Intent(n) => n.to_string(),
            Target::Query(e) => format!("list:{e}"),
        };
        self.audit.push(AuditEntry {
            time: now,
            agent: r.agent.into(),
            app_id: r.app_id.into(),
            target,
            outcome,
        });
    }

    pub fn decide(&mut self, r: &Request, now: u64) -> Outcome {
        let outcome = self.decide_inner(r, now);
        self.log(
            now,
            r,
            match &outcome {
                Outcome::Run(_) => "run",
                Outcome::Ask(_) => "ask",
                Outcome::Refuse(Refusal::Denied) => "refused-denied",
                Outcome::Refuse(Refusal::BadArguments(_)) => "refused-arguments",
                Outcome::Refuse(_) => "refused",
            },
        );
        outcome
    }

    fn decide_inner(&mut self, r: &Request, now: u64) -> Outcome {
        let Some(m) = self.manifests.get(r.app_id).cloned() else {
            return Outcome::Refuse(Refusal::UnknownApp);
        };
        let hash = m["hash"].as_str().unwrap_or("").to_string();
        let (scope, risk, payload, about, name) = match r.target {
            Target::Intent(n) => {
                let Some(i) = m["intents"].get(n) else {
                    return Outcome::Refuse(Refusal::UnknownTarget);
                };
                let payload = match validate_args(&m, n, &r.args) {
                    Ok(p) => p,
                    Err(e) => return Outcome::Refuse(Refusal::BadArguments(e)),
                };
                let Some(risk) = i["risk"].as_str().and_then(Risk::parse) else {
                    return Outcome::Refuse(Refusal::UnknownTarget);
                };
                (
                    i["scope"].as_str().unwrap_or("").to_string(),
                    risk,
                    payload,
                    i["about"].as_str().unwrap_or("").to_string(),
                    n.to_string(),
                )
            }
            Target::Query(e) => {
                let Some(ent) = m["entities"].get(e) else {
                    return Outcome::Refuse(Refusal::UnknownTarget);
                };
                (
                    ent["scope"].as_str().unwrap_or("").to_string(),
                    Risk::Read,
                    None,
                    format!("List {e} records"),
                    format!("list:{e}"),
                )
            }
        };

        if self.saved.denials.iter().any(|d| {
            d.agent == r.agent && d.app_id == r.app_id && d.scope == scope && d.until > now
        }) {
            return Outcome::Refuse(Refusal::Denied);
        }
        let fingerprint = sha256_hex(
            canonical(&json!({ "target": name, "args": r.args, "shown": r.shown })).as_bytes(),
        );
        let live = self.saved.grants.iter().position(|g| {
            g.agent == r.agent
                && g.app_id == r.app_id
                && g.scope == scope
                && g.manifest == hash
                && g.expires.is_none_or(|t| t > now)
        });
        if let Some(i) = live {
            if !risk.per_action() {
                match self.saved.grants[i].uses_left {
                    Some(1) => {
                        self.saved.grants.remove(i);
                        let _ = self.save();
                    }
                    Some(n) => self.saved.grants[i].uses_left = Some(n - 1),
                    None => {}
                }
                return Outcome::Run(payload);
            }
            if self.saved.grants[i].widened {
                return Outcome::Run(payload);
            }
        }
        if risk.per_action() {
            self.approvals.retain(|a| a.until > now);
            if let Some(pos) = self.approvals.iter().position(|a| {
                a.agent == r.agent
                    && a.app_id == r.app_id
                    && a.fingerprint == fingerprint
                    && a.manifest == hash
            }) {
                self.approvals.remove(pos);
                return Outcome::Run(payload);
            }
        }

        let id = self.next_prompt;
        self.next_prompt += 1;
        let choices = if risk.per_action() {
            vec![Answer::Deny, Answer::Once, Answer::Widen]
        } else {
            vec![
                Answer::Deny,
                Answer::Once,
                Answer::For(3600),
                Answer::Always,
            ]
        };
        let prompt = Prompt {
            id,
            agent: r.agent.into(),
            app_id: r.app_id.into(),
            app_name: m["app"]["name"].as_str().unwrap_or("").into(),
            scope_about: m["scopes"][&scope]["about"].as_str().unwrap_or("").into(),
            scope,
            risk,
            target: name,
            about,
            arguments: r.args.clone(),
            shown: r.shown.clone(),
            choices,
        };
        self.pending.insert(
            id,
            Pending {
                prompt: prompt.clone(),
                fingerprint,
                manifest: hash,
            },
        );
        Outcome::Ask(Box::new(prompt))
    }

    /// The owner's answer to a prompt. Only a prompt this broker issued, once, can be answered.
    pub fn answer(&mut self, prompt_id: u64, answer: Answer, now: u64) -> Result<(), Invalid> {
        let Some(p) = self.pending.remove(&prompt_id) else {
            return Err(Invalid("no such prompt"));
        };
        if !p.prompt.choices.contains(&answer) && !matches!(answer, Answer::For(_)) {
            self.pending.insert(prompt_id, p);
            return Err(Invalid("that choice is not offered for this request"));
        }
        if matches!(answer, Answer::For(_)) && p.prompt.risk.per_action() {
            self.pending.insert(prompt_id, p);
            return Err(Invalid(
                "external and privileged requests are approved per action",
            ));
        }
        let pr = &p.prompt;
        let (agent, app_id, scope) = (pr.agent.clone(), pr.app_id.clone(), pr.scope.clone());
        let grant = |expires: Option<u64>, widened: bool, uses_left: Option<u32>| Grant {
            agent: agent.clone(),
            app_id: app_id.clone(),
            scope: scope.clone(),
            manifest: p.manifest.clone(),
            expires,
            widened,
            uses_left,
        };
        let same = |g: &Grant| g.agent == agent && g.app_id == app_id && g.scope == scope;
        let new_grant = match answer {
            Answer::Deny => {
                self.saved.denials.push(Denial {
                    agent: agent.clone(),
                    app_id: app_id.clone(),
                    scope: scope.clone(),
                    until: now + DENIAL_SECS,
                });
                None
            }
            Answer::Once if pr.risk.per_action() => {
                self.approvals.push(Approval {
                    agent: agent.clone(),
                    app_id: app_id.clone(),
                    fingerprint: p.fingerprint.clone(),
                    manifest: p.manifest.clone(),
                    until: now + APPROVAL_SECS,
                });
                None
            }
            Answer::Once => Some(grant(None, false, Some(1))),
            Answer::For(secs) => Some(grant(Some(now + secs), false, None)),
            Answer::Always => Some(grant(None, false, None)),
            Answer::Widen => Some(grant(None, true, None)),
        };
        if answer != Answer::Deny {
            self.saved
                .denials
                .retain(|d| !(d.agent == agent && d.app_id == app_id && d.scope == scope));
        }
        if let Some(g) = new_grant {
            // A new grant replaces an older one for the same agent, app and scope.
            self.saved.grants.retain(|old| !same(old));
            self.saved.grants.push(g);
        }
        let _ = self.save();
        Ok(())
    }

    /// Withdraws every grant (and per-action approval) an agent holds, for one app or all. Immediate.
    pub fn revoke(&mut self, agent: &str, app_id: Option<&str>) -> usize {
        let before = self.saved.grants.len();
        self.saved
            .grants
            .retain(|g| !(g.agent == agent && app_id.is_none_or(|a| a == g.app_id)));
        self.approvals
            .retain(|a| !(a.agent == agent && app_id.is_none_or(|x| x == a.app_id)));
        let _ = self.save();
        before - self.saved.grants.len()
    }

    /// Who holds what, for the Settings page: `(agent, app id, scope, widened, expires)`.
    pub fn grants(&self, now: u64) -> Vec<(String, String, String, bool, Option<u64>)> {
        self.saved
            .grants
            .iter()
            .filter(|g| g.expires.is_none_or(|t| t > now))
            .map(|g| {
                (
                    g.agent.clone(),
                    g.app_id.clone(),
                    g.scope.clone(),
                    g.widened,
                    g.expires,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
