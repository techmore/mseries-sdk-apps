//! The Smart Reply input service: an IBus engine that holds one drafted reply for the focused field.
//!
//! The trigger ([`crate::trigger`]) hands a draft to `org.techmore.SmartReplyInput.Offer`. The service checks that an
//! editable, non-sensitive field has focus (the same checks as the F5 dictation input), switches to its own engine so
//! it sees the next keys, and holds the draft. **Tab** commits it once with `CommitText`; **Esc**, any other key, focus
//! leaving the field or 60 seconds discard it. The previous input method is restored afterwards. Nothing is ever sent:
//! the engine commits text, never Enter. F5 dictation is a separate engine and is not touched.

use crate::a11y::{A11y, NodeRef};
use crate::held::{self, Held, KeyAction};
use crate::tree::{self, Limits};
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, StructureBuilder, Value};
use zbus::{Connection, block_on, connection::Builder, interface};

pub const SERVICE_NAME: &str = "org.techmore.SmartReplyInput";
pub const SERVICE_PATH: &str = "/org/techmore/SmartReplyInput";
pub const ENGINE_NAME: &str = "mseries-smart-reply";
const COMPONENT_NAME: &str = "org.freedesktop.IBus.MSeriesSmartReply";
const FACTORY_PATH: &str = "/org/freedesktop/IBus/Factory";
const IBUS_PATH: &str = "/org/freedesktop/IBus";
const IBUS_NAME: &str = "org.freedesktop.IBus";
const RELEASE_MASK: u32 = 1 << 30;
/// How long an offer may wait for the app to attach to the engine (the Python service allowed 2.75 s).
const ATTACH_DEADLINE: Duration = Duration::from_millis(2750);

// ---- IBus wire format -------------------------------------------------------------------------------------------
// IBus serialises its objects as `(s a{sv} fields...)`: the type name, an attachments dictionary, then the fields.

fn no_attachments() -> Value<'static> {
    Value::new(HashMap::<String, Value<'static>>::new())
}

fn variants(items: Vec<Value<'static>>) -> Value<'static> {
    let mut array = zbus::zvariant::Array::new(&zbus::zvariant::Signature::Variant);
    for item in items {
        array
            .append(Value::Value(Box::new(item)))
            .expect("variants of any type fit an av array");
    }
    Value::Array(array)
}

fn strings(builder: StructureBuilder<'static>, fields: &[&str]) -> StructureBuilder<'static> {
    fields.iter().fold(builder, |builder, field| {
        builder.add_field(field.to_string())
    })
}

/// `IBusText`: `(sa{sv}sv)` with an empty `IBusAttrList` `(sa{sv}av)`.
pub fn ibus_text(text: &str) -> Value<'static> {
    let attributes = StructureBuilder::new()
        .add_field("IBusAttrList".to_string())
        .append_field(no_attachments())
        .append_field(variants(Vec::new()))
        .build()
        .expect("static structure");
    StructureBuilder::new()
        .add_field("IBusText".to_string())
        .append_field(no_attachments())
        .add_field(text.to_string())
        .append_field(Value::Value(Box::new(Value::Structure(attributes))))
        .build()
        .map(Value::Structure)
        .expect("static structure")
}

/// `IBusEngineDesc`: `(sa{sv}` name, longname, description, language, license, author, icon, layout, `u` rank, then
/// hotkeys, symbol, setup, layout variant, layout option, version, textdomain, icon property key `)`.
pub fn engine_desc() -> Value<'static> {
    let builder = StructureBuilder::new()
        .add_field("IBusEngineDesc".to_string())
        .append_field(no_attachments());
    let builder = strings(
        builder,
        &[
            ENGINE_NAME,
            "Smart Reply",
            "Holds one drafted reply; Tab inserts it",
            "en",
            "GPL-3.0-only",
            "ElementaryOS MSeries contributors",
            "mail-reply-sender",
            "default",
        ],
    )
    .add_field(0_u32);
    let builder = strings(builder, &["", "", "", "", "", "", "", ""]);
    Value::Structure(builder.build().expect("static structure"))
}

/// `IBusComponent`: `(sa{sv}` name, description, version, license, author, homepage, command line, textdomain,
/// `av` observed paths, `av` engines `)`.
pub fn component() -> Value<'static> {
    let builder = StructureBuilder::new()
        .add_field("IBusComponent".to_string())
        .append_field(no_attachments());
    let builder = strings(
        builder,
        &[
            COMPONENT_NAME,
            "MSeries Smart Reply input",
            env!("CARGO_PKG_VERSION"),
            "GPL-3.0-only",
            "ElementaryOS MSeries contributors",
            "https://github.com/techmore/ElementaryOS_MSeries",
            "",
            "",
        ],
    )
    .append_field(variants(Vec::new()))
    .append_field(variants(vec![engine_desc()]));
    Value::Structure(builder.build().expect("static structure"))
}

/// The name of an `IBusEngineDesc` returned by the daemon (a variant, possibly wrapped in another).
pub fn engine_desc_name(value: &Value<'_>) -> Option<String> {
    match value {
        Value::Value(inner) => engine_desc_name(inner),
        Value::Structure(structure) => {
            let fields = structure.fields();
            match fields.first() {
                Some(Value::Str(kind)) if kind.as_str() == "IBusEngineDesc" => {}
                _ => return None,
            }
            match fields.get(2) {
                Some(Value::Str(name)) if !name.is_empty() => Some(name.to_string()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The input method to return to when the lease ends, recovered from the desktop's input sources when IBus lost its
/// global engine. Pantheon's XKB sources use IBus's US pass-through engine (the dictation input's `native_engine`).
pub fn native_engine(sources: &[(String, String)], index: usize) -> Option<String> {
    let (kind, name) = sources.get(index)?;
    if name.is_empty() || name.len() > 256 {
        return None;
    }
    match kind.as_str() {
        "xkb" => Some("xkb:us::eng".into()),
        "ibus" => Some(name.clone()),
        _ => None,
    }
}

/// Parses `gsettings get org.gnome.desktop.input-sources sources`, for example `[('xkb', 'us'), ('ibus', 'anthy')]`.
pub fn parse_sources(text: &str) -> Vec<(String, String)> {
    let mut sources = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('(') {
        let Some(end) = rest[start..].find(')') else {
            break;
        };
        let inner = &rest[start + 1..start + end];
        let parts: Vec<&str> = inner
            .split(',')
            .map(|part| part.trim().trim_matches('\''))
            .collect();
        if let [kind, name] = parts[..] {
            sources.push((kind.to_string(), name.to_string()));
        }
        rest = &rest[start + end + 1..];
    }
    sources
}

fn gsettings(key: &str) -> Option<String> {
    let output = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.input-sources", key])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn configured_native_engine() -> Option<String> {
    let sources = parse_sources(&gsettings("sources")?);
    let current = gsettings("current")?
        .trim_start_matches("uint32 ")
        .parse()
        .ok()?;
    native_engine(&sources, current)
}

fn ibus_address() -> Result<String, String> {
    if let Some(address) = std::env::var("IBUS_ADDRESS").ok().filter(|a| !a.is_empty()) {
        return Ok(address);
    }
    let output = Command::new("ibus")
        .arg("address")
        .output()
        .map_err(|error| format!("IBus is unavailable: {error}"))?;
    let address = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() && !address.is_empty() {
        Ok(address)
    } else {
        Err("IBus is unavailable".into())
    }
}

// ---- Shared state -----------------------------------------------------------------------------------------------

#[derive(Default)]
struct State {
    held: Held,
    /// The engine the app attached to (it has focus), if any.
    engine: Option<OwnedObjectPath>,
    busy: bool,
    previous: Option<String>,
    epoch: u64,
    /// The field the draft belongs to.
    identity: Option<NodeRef>,
    /// Keys we consumed whose release must not reach the app.
    swallow: HashSet<u32>,
    /// `(purpose, hints)` the app reported for the field.
    content_type: (u32, u32),
    engines: u32,
}

struct Shared {
    state: Mutex<State>,
    ibus: OnceLock<Connection>,
    a11y: Mutex<Option<Arc<A11y>>>,
}

type Share = Arc<Shared>;

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn ibus(&self) -> Result<&Connection, String> {
        self.ibus
            .get()
            .ok_or_else(|| "IBus is not connected".into())
    }

    fn a11y(&self) -> Result<Arc<A11y>, String> {
        let mut slot = self.a11y.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(a11y) = slot.as_ref() {
            return Ok(a11y.clone());
        }
        let a11y = Arc::new(A11y::connect()?);
        *slot = Some(a11y.clone());
        Ok(a11y)
    }

    /// Drops the cached accessibility connection so the next use reconnects.
    fn forget_a11y(&self) {
        *self.a11y.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// The focused field, if it is an editable, non-sensitive field in the active window.
    fn focused_field(&self) -> Result<NodeRef, String> {
        let a11y = self.a11y()?;
        let node = tree::focused_node(&*a11y, Limits::FOCUS).ok_or_else(|| {
            self.forget_a11y();
            "No accessible focused field".to_string()
        })?;
        tree::check_field(&*a11y, &node, Limits::FOCUS).map_err(str::to_string)?;
        Ok(node)
    }

    /// Whether this exact field still has focus and is still safe.
    fn field_still_safe(&self, identity: &NodeRef) -> bool {
        self.a11y()
            .is_ok_and(|a11y| tree::check_field(&*a11y, identity, Limits::FOCUS).is_ok())
    }

    /// Ends the draft: forgets it, hides the hint and gives the input method back. Does not wait for IBus.
    fn discard(self: &Arc<Self>) {
        let (had, engine, previous) = {
            let mut state = self.lock();
            let had = state.held.is_holding();
            state.held.drop_draft();
            state.epoch += 1;
            state.identity = None;
            (had, state.engine.clone(), state.previous.take())
        };
        let Ok(ibus) = self.ibus() else { return };
        if had && let Some(path) = engine {
            let hint = ibus.clone();
            ibus.executor()
                .spawn(
                    async move {
                        if let Ok(emitter) = SignalEmitter::new(&hint, path) {
                            let _ = Engine::hide_auxiliary_text(&emitter).await;
                        }
                    },
                    "smart-reply-hide-hint",
                )
                .detach();
        }
        if let Some(previous) = previous {
            restore_engine(ibus.clone(), previous);
        }
    }
}

/// Selects `previous` again, but only while our own engine is still the global one.
fn restore_engine(ibus: Connection, previous: String) {
    ibus.clone()
        .executor()
        .spawn(
            async move {
                if previous == ENGINE_NAME {
                    return;
                }
                if current_engine(&ibus).await.as_deref() == Some(ENGINE_NAME) {
                    let _ = ibus
                        .call_method(
                            Some(IBUS_NAME),
                            IBUS_PATH,
                            Some(IBUS_NAME),
                            "SetGlobalEngine",
                            &(previous.as_str(),),
                        )
                        .await;
                }
            },
            "smart-reply-restore-engine",
        )
        .detach();
}

async fn current_engine(ibus: &Connection) -> Option<String> {
    let reply = ibus
        .call_method(
            Some(IBUS_NAME),
            IBUS_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(IBUS_NAME, "GlobalEngine"),
        )
        .await
        .ok()?;
    let value: OwnedValue = reply.body().deserialize().ok()?;
    engine_desc_name(&value)
}

// ---- The engine and its factory (objects on the IBus connection) --------------------------------------------------

struct Engine {
    shared: Share,
}

#[interface(name = "org.freedesktop.IBus.Engine")]
impl Engine {
    async fn process_key_event(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        keyval: u32,
        _keycode: u32,
        state: u32,
    ) -> bool {
        let released = state & RELEASE_MASK != 0;
        let now = Instant::now();
        let action = {
            let mut guard = self.shared.lock();
            if released && guard.swallow.remove(&keyval) {
                return true;
            }
            held::classify_key(&guard.held, keyval, released, now)
        };
        match action {
            KeyAction::Insert => {
                let (text, identity, content_type) = {
                    let mut guard = self.shared.lock();
                    (
                        guard.held.take(now),
                        guard.identity.clone(),
                        guard.content_type,
                    )
                };
                let safe = match (&text, &identity) {
                    (Some(_), Some(identity)) => {
                        // The app may have moved on since the offer; the draft belongs to one field only.
                        content_type_ok(content_type) && self.shared.field_still_safe(identity)
                    }
                    _ => false,
                };
                self.shared.lock().swallow.insert(keyval);
                if !safe {
                    eprintln!("smart-reply: Tab ignored; the draft expired or the field changed");
                }
                if let (true, Some(text)) = (safe, text) {
                    let _ = Self::hide_auxiliary_text(&emitter).await;
                    let _ = Self::commit_text(&emitter, ibus_text(&text)).await;
                }
                self.shared.discard();
                true
            }
            KeyAction::DiscardConsume => {
                self.shared.lock().swallow.insert(keyval);
                self.shared.discard();
                true
            }
            KeyAction::DiscardPass => {
                self.shared.discard();
                false
            }
            KeyAction::Swallow => true,
            KeyAction::Ignore => false,
        }
    }

    fn focus_in(&self, #[zbus(header)] _header: zbus::message::Header<'_>) {}

    fn focus_out(&self) {
        self.lost_focus(None);
    }

    fn focus_in_id(
        &self,
        #[zbus(object_server)] _server: &zbus::ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
        _object_path: &str,
        _client: &str,
    ) {
        if let Some(path) = header.path() {
            self.shared.lock().engine = Some(path.clone().into());
        }
    }

    fn focus_out_id(&self, _object_path: &str) {
        self.lost_focus(None);
    }

    fn reset(&self) {}
    fn enable(&self) {}
    fn disable(&self) {
        self.shared.lock().engine = None;
    }
    fn set_capabilities(&self, _capabilities: u32) {}
    fn set_cursor_location(&self, _x: i32, _y: i32, _w: i32, _h: i32) {}
    fn set_surrounding_text(&self, _text: Value<'_>, _cursor: u32, _anchor: u32) {}
    fn property_activate(&self, _name: &str, _state: u32) {}
    fn property_show(&self, _name: &str) {}
    fn property_hide(&self, _name: &str) {}
    fn candidate_clicked(&self, _index: u32, _button: u32, _state: u32) {}
    fn page_up(&self) {}
    fn page_down(&self) {}
    fn cursor_up(&self) {}
    fn cursor_down(&self) {}
    fn panel_extension_received(&self, _event: Value<'_>) {}
    fn panel_extension_register_keys(&self, _keys: Value<'_>) {}

    #[zbus(property)]
    fn focus_id(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn content_type(&self) -> (u32, u32) {
        self.shared.lock().content_type
    }

    #[zbus(property)]
    fn set_content_type(&self, value: (u32, u32)) {
        self.shared.lock().content_type = value;
    }

    #[zbus(property)]
    fn active_surrounding_text(&self) -> bool {
        false
    }

    #[zbus(signal)]
    async fn commit_text(emitter: &SignalEmitter<'_>, text: Value<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn update_auxiliary_text(
        emitter: &SignalEmitter<'_>,
        text: Value<'_>,
        visible: bool,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn hide_auxiliary_text(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

impl Engine {
    /// The app moved focus away. A held draft belongs to the field it was offered for, so it goes.
    fn lost_focus(&self, _path: Option<&str>) {
        let discard = {
            let mut state = self.shared.lock();
            state.engine = None;
            state.held.is_holding() && !state.busy
        };
        if discard {
            self.shared.discard();
        }
    }
}

/// The app reported a password or PIN purpose, or asked for no learning/logging (sensitive data hint).
fn content_type_ok((purpose, hints): (u32, u32)) -> bool {
    const PURPOSE_PASSWORD: u32 = 8;
    const PURPOSE_PIN: u32 = 9;
    const HINT_PRIVATE: u32 = 2048;
    !matches!(purpose, PURPOSE_PASSWORD | PURPOSE_PIN) && hints & HINT_PRIVATE == 0
}

struct EngineService;

#[interface(name = "org.freedesktop.IBus.Service")]
impl EngineService {
    fn destroy(&self) {}
}

struct Factory {
    shared: Share,
}

#[interface(name = "org.freedesktop.IBus.Factory")]
impl Factory {
    async fn create_engine(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        name: &str,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        if name != ENGINE_NAME {
            return Err(zbus::fdo::Error::InvalidArgs("Unknown engine".into()));
        }
        let number = {
            let mut state = self.shared.lock();
            state.engines += 1;
            state.engines
        };
        let path = OwnedObjectPath::try_from(format!(
            "/org/freedesktop/IBus/MSeriesSmartReply/Engine{number}"
        ))
        .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        server
            .at(
                &path,
                Engine {
                    shared: self.shared.clone(),
                },
            )
            .await?;
        server.at(&path, EngineService).await?;
        Ok(path)
    }
}

// ---- The session-bus interface the trigger calls ----------------------------------------------------------------

struct SmartReplyInput {
    shared: Share,
}

fn unavailable(message: impl Into<String>) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(message.into())
}

#[interface(name = "org.techmore.SmartReplyInput")]
impl SmartReplyInput {
    async fn offer(&self, text: String) -> zbus::fdo::Result<()> {
        offer(&self.shared, &text).await.map_err(unavailable)
    }

    fn cancel(&self) {
        self.shared.discard();
    }

    fn ready(&self) -> bool {
        true
    }
}

async fn offer(shared: &Share, text: &str) -> Result<(), String> {
    if !held::valid_offer(text) {
        return Err("Invalid draft".into());
    }
    if shared.lock().busy || shared.lock().held.is_holding() {
        shared.discard();
    }
    // Refuse sensitive fields BEFORE switching input methods.
    let identity = shared
        .focused_field()
        .map_err(|_| "Accessible field metadata unavailable or unsafe".to_string())?;
    let ibus = shared.ibus()?.clone();
    let previous = match current_engine(&ibus).await {
        Some(name) => Some(name),
        None => configured_native_engine(),
    }
    .ok_or("No current or configured native input method")?;

    let epoch = {
        let mut state = shared.lock();
        state.busy = true;
        state.previous = Some(previous);
        state.epoch += 1;
        state.identity = Some(identity.clone());
        state.epoch
    };
    let fail = |message: &str| {
        let message = message.to_string();
        shared.lock().busy = false;
        shared.discard();
        message
    };

    if ibus
        .call_method(
            Some(IBUS_NAME),
            IBUS_PATH,
            Some(IBUS_NAME),
            "SetGlobalEngine",
            &(ENGINE_NAME,),
        )
        .await
        .is_err()
    {
        return Err(fail("Could not select the Smart Reply input method"));
    }

    let deadline = Instant::now() + ATTACH_DEADLINE;
    loop {
        if Instant::now() > deadline {
            return Err(fail("Smart Reply startup deadline exceeded"));
        }
        if shared.lock().epoch != epoch {
            return Err(fail("Smart Reply was cancelled"));
        }
        if !shared.field_still_safe(&identity) {
            return Err(fail("Text-field focus changed"));
        }
        let engine = shared.lock().engine.clone();
        if let Some(path) = engine {
            let held = shared.lock().held.hold(text, Instant::now());
            if held.is_err() {
                return Err(fail("Invalid draft"));
            }
            shared.lock().busy = false;
            if let Ok(emitter) = SignalEmitter::new(&ibus, path) {
                let _ = Engine::update_auxiliary_text(
                    &emitter,
                    ibus_text("Tab inserts the draft, Esc discards it"),
                    true,
                )
                .await;
            }
            let expiring = shared.clone();
            ibus.executor()
                .spawn(
                    async move {
                        async_io::Timer::after(held::HOLD).await;
                        let still = {
                            let state = expiring.lock();
                            state.epoch == epoch && state.held.is_holding()
                        };
                        if still {
                            expiring.discard();
                        }
                    },
                    "smart-reply-expire",
                )
                .detach();
            return Ok(());
        }
        async_io::Timer::after(Duration::from_millis(25)).await;
    }
}

/// Runs the service until IBus goes away (the unit restarts it).
pub fn serve() -> Result<(), String> {
    let shared: Share = Arc::new(Shared {
        state: Mutex::new(State::default()),
        ibus: OnceLock::new(),
        a11y: Mutex::new(None),
    });
    block_on(async {
        let address = ibus_address()?;
        let ibus = Builder::address(address.as_str())
            .map_err(|error| error.to_string())?
            .method_timeout(Duration::from_millis(2000))
            .serve_at(
                FACTORY_PATH,
                Factory {
                    shared: shared.clone(),
                },
            )
            .map_err(|error| error.to_string())?
            .build()
            .await
            .map_err(|error| format!("could not connect to IBus: {error}"))?;
        shared
            .ibus
            .set(ibus.clone())
            .map_err(|_| "IBus connection set twice".to_string())?;
        ibus.call_method(
            Some(IBUS_NAME),
            IBUS_PATH,
            Some(IBUS_NAME),
            "RegisterComponent",
            &(component(),),
        )
        .await
        .map_err(|error| format!("IBus refused the Smart Reply component: {error}"))?;

        let _session = Builder::session()
            .map_err(|error| error.to_string())?
            .name(SERVICE_NAME)
            .map_err(|error| error.to_string())?
            .serve_at(
                SERVICE_PATH,
                SmartReplyInput {
                    shared: shared.clone(),
                },
            )
            .map_err(|error| error.to_string())?
            .build()
            .await
            .map_err(|error| format!("could not own {SERVICE_NAME}: {error}"))?;

        // Exit (and be restarted) when IBus goes away.
        let mut failures = 0;
        loop {
            async_io::Timer::after(Duration::from_secs(15)).await;
            let ping = ibus
                .call_method(
                    Some(IBUS_NAME),
                    IBUS_PATH,
                    Some("org.freedesktop.DBus.Peer"),
                    "Ping",
                    &(),
                )
                .await;
            failures = if ping.is_ok() { 0 } else { failures + 1 };
            if failures >= 2 {
                return Err("IBus went away".to_string());
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_objects_have_the_signatures_ibus_expects() {
        assert_eq!(ibus_text("hi").value_signature().to_string(), "(sa{sv}sv)");
        assert_eq!(
            engine_desc().value_signature().to_string(),
            "(sa{sv}ssssssssussssssss)"
        );
        assert_eq!(
            component().value_signature().to_string(),
            "(sa{sv}ssssssssavav)"
        );
    }

    #[test]
    fn the_engine_name_is_read_through_nested_variants() {
        let wrapped = Value::Value(Box::new(Value::Value(Box::new(engine_desc()))));
        assert_eq!(engine_desc_name(&wrapped).as_deref(), Some(ENGINE_NAME));
        assert_eq!(engine_desc_name(&ibus_text("x")), None);
        assert_eq!(engine_desc_name(&Value::from(1_u32)), None);
    }

    #[test]
    fn the_previous_input_method_is_recovered_from_the_desktop_sources() {
        let sources = parse_sources("[('xkb', 'us'), ('ibus', 'anthy')]");
        assert_eq!(sources.len(), 2);
        assert_eq!(native_engine(&sources, 0).as_deref(), Some("xkb:us::eng"));
        assert_eq!(native_engine(&sources, 1).as_deref(), Some("anthy"));
        assert_eq!(native_engine(&sources, 2), None);
        assert_eq!(native_engine(&[("other".into(), "x".into())], 0), None);
        assert_eq!(native_engine(&[("ibus".into(), String::new())], 0), None);
        assert!(parse_sources("@a(ss) []").is_empty());
    }

    #[test]
    fn password_and_private_content_types_are_refused() {
        assert!(content_type_ok((0, 0)));
        assert!(content_type_ok((1, 0)));
        assert!(!content_type_ok((8, 0)));
        assert!(!content_type_ok((9, 0)));
        assert!(!content_type_ok((0, 2048)));
    }
}
