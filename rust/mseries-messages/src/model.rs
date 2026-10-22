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
}

impl Thread {
    pub fn unread(&self) -> usize {
        self.messages
            .iter()
            .filter(|message| !message.outgoing && !message.read)
            .count()
            .max(self.unread_count)
    }

    pub fn preview(&self) -> &str {
        self.messages
            .last()
            .map_or("", |message| message.body.as_str())
    }
}
