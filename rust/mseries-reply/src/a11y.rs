//! The AT-SPI transport: the accessibility bus as a [`Tree`]. Every call has a hard timeout, so an unresponsive app
//! costs one timeout and never a hang. This module only moves bytes; the bounds and the rules live in [`crate::tree`]
//! and [`crate::context`].

use crate::tree::{States, Tree};
use std::time::Duration;
use zbus::zvariant::{DynamicType, ObjectPath, OwnedValue, Structure};
use zbus::{Connection, block_on};

const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT: &str = "/org/a11y/atspi/accessible/root";
const NULL_PATH: &str = "/org/a11y/atspi/null";
const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const TEXT: &str = "org.a11y.atspi.Text";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// One call to one app. The Python bridge used 200 ms (and 500 ms for the first call to an app).
const CALL_TIMEOUT: Duration = Duration::from_millis(300);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct NodeRef {
    pub bus: String,
    pub path: String,
}

pub struct A11y {
    connection: Connection,
}

impl A11y {
    /// Connects to the accessibility bus the session announces (`org.a11y.Bus`).
    pub fn connect() -> Result<Self, String> {
        block_on(async {
            let session = Connection::session()
                .await
                .map_err(|error| format!("no session bus: {error}"))?;
            let reply = session
                .call_method(
                    Some("org.a11y.Bus"),
                    "/org/a11y/bus",
                    Some("org.a11y.Bus"),
                    "GetAddress",
                    &(),
                )
                .await
                .map_err(|error| format!("the accessibility bus is not available: {error}"))?;
            let address: String = reply
                .body()
                .deserialize()
                .map_err(|error| error.to_string())?;
            let connection = zbus::connection::Builder::address(address.as_str())
                .map_err(|error| error.to_string())?
                .method_timeout(CALL_TIMEOUT)
                .build()
                .await
                .map_err(|error| format!("could not join the accessibility bus: {error}"))?;
            Ok(Self { connection })
        })
    }

    /// The toolkit the application behind `node` reports (`gtk`, `Qt`, `Chromium`...).
    pub fn toolkit(&self, node: &NodeRef) -> Option<String> {
        let root = NodeRef {
            bus: node.bus.clone(),
            path: ROOT.into(),
        };
        String::try_from(self.property(&root, "org.a11y.atspi.Application", "ToolkitName")?).ok()
    }

    async fn call_async<B>(
        &self,
        node: &NodeRef,
        interface: &str,
        method: &str,
        body: &B,
    ) -> Option<zbus::Message>
    where
        B: serde::Serialize + DynamicType,
    {
        let path = ObjectPath::try_from(node.path.as_str()).ok()?;
        self.connection
            .call_method(Some(node.bus.as_str()), path, Some(interface), method, body)
            .await
            .ok()
    }

    fn call<B>(
        &self,
        node: &NodeRef,
        interface: &str,
        method: &str,
        body: &B,
    ) -> Option<zbus::Message>
    where
        B: serde::Serialize + DynamicType,
    {
        block_on(self.call_async(node, interface, method, body))
    }

    fn property(&self, node: &NodeRef, interface: &str, name: &str) -> Option<OwnedValue> {
        self.call(node, PROPERTIES, "Get", &(interface, name))
            .and_then(|reply| reply.body().deserialize::<OwnedValue>().ok())
    }

    fn reference(bus: String, path: String) -> Option<NodeRef> {
        (!bus.is_empty() && path != NULL_PATH && !path.is_empty()).then_some(NodeRef { bus, path })
    }
}

impl Tree for A11y {
    type Node = NodeRef;

    fn applications(&self) -> Vec<NodeRef> {
        let root = NodeRef {
            bus: REGISTRY.into(),
            path: ROOT.into(),
        };
        self.children(&root, 128)
    }

    fn children(&self, node: &NodeRef, limit: usize) -> Vec<NodeRef> {
        self.call(node, ACCESSIBLE, "GetChildren", &())
            .and_then(|reply| {
                reply
                    .body()
                    .deserialize::<Vec<(String, zbus::zvariant::OwnedObjectPath)>>()
                    .ok()
            })
            .map(|children| {
                children
                    .into_iter()
                    .take(limit)
                    .filter_map(|(bus, path)| Self::reference(bus, path.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn role(&self, node: &NodeRef) -> Option<u32> {
        self.call(node, ACCESSIBLE, "GetRole", &())?
            .body()
            .deserialize::<u32>()
            .ok()
    }

    fn states(&self, node: &NodeRef) -> Option<States> {
        let words = self
            .call(node, ACCESSIBLE, "GetState", &())?
            .body()
            .deserialize::<Vec<u32>>()
            .ok()?;
        Some(States::from_words(&words))
    }

    fn parent(&self, node: &NodeRef) -> Option<NodeRef> {
        let value = self.property(node, ACCESSIBLE, "Parent")?;
        let structure = Structure::try_from(value).ok()?;
        let fields = structure.fields();
        let bus = String::try_from(fields.first()?.try_clone().ok()?).ok()?;
        let path = zbus::zvariant::OwnedObjectPath::try_from(fields.get(1)?.try_clone().ok()?)
            .ok()?
            .to_string();
        Self::reference(bus, path)
    }

    fn name(&self, node: &NodeRef) -> Option<String> {
        String::try_from(self.property(node, ACCESSIBLE, "Name")?).ok()
    }

    fn text(&self, node: &NodeRef, max_chars: usize) -> Option<String> {
        // Prefer the Text interface; fall back to the accessible name when the node has none.
        let count = self
            .property(node, TEXT, "CharacterCount")
            .and_then(|value| i32::try_from(value).ok());
        if let Some(count) = count {
            let end = count.clamp(0, max_chars as i32);
            let reply = self.call(node, TEXT, "GetText", &(0_i32, end))?;
            return reply.body().deserialize::<String>().ok();
        }
        self.name(node)
    }
}
