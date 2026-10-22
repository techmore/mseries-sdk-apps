use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub struct Message {
    pub handle: String,
    pub body: String,
    pub timestamp: String,
    pub outgoing: bool,
    pub read: bool,
    pub sender: String,
    pub body_truncated: bool,
    pub phone_list: bool,
}

impl Message {
    pub fn visible_body(&self) -> String {
        if self.has_attachment() {
            self.body.replace('\u{fffc}', "").trim().to_string()
        } else {
            self.body.clone()
        }
    }

    pub fn has_attachment(&self) -> bool {
        self.body.contains('\u{fffc}') || self.body.trim().is_empty()
    }

    pub fn is_shortened(&self) -> bool {
        let visible = self.visible_body();
        self.body_truncated || (self.phone_list && visible.chars().count() >= 120)
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub struct Thread {
    pub key: String,
    pub name: String,
    pub is_group: bool,
    pub recipients: Vec<String>,
    pub reply_ready: bool,
    pub messages: Vec<Message>,
    pub last_ts: String,
    pub participants_required: bool,
    pub roster_changed: bool,
    pub confirmation_token: String,
    pub starred: bool,
    pub group_confirmed: bool,
    pub unread_count: usize,
    #[serde(skip)]
    pub local_only: bool,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub struct ContactMatch {
    pub name: String,
    pub address: String,
}

impl Thread {
    pub fn unread(&self) -> usize {
        self.messages
            .iter()
            .filter(|message| !message.outgoing && !message.read)
            .count()
            .max(self.unread_count)
    }

    pub fn preview(&self) -> String {
        let Some(message) = self.messages.last() else {
            return String::new();
        };
        let text = message
            .visible_body()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if message.has_attachment() {
            if text.is_empty() {
                "Attachment".to_string()
            } else {
                format!("{text} · Attachment")
            }
        } else {
            text
        }
    }
}
