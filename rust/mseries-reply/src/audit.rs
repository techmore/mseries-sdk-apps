//! What is recorded about a request: tiers, sizes and outcome. Never a message body, a draft, a contact name or a window
//! title, and a refusal records only its kind, so a blocked contact leaves no trace of who it was. The owner's personal
//! records that went into a prompt appear only as counts (how many of each kind, and how many characters).

use crate::context::{Packet, Refusal, Tier};
use mseries_ai::RetrievalCounts;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct AuditRecord {
    pub app_id: String,
    pub outcome: &'static str,
    pub tiers: Vec<Tier>,
    pub truncated: bool,
    pub local_only: bool,
    pub personal: RetrievalCounts,
}

impl AuditRecord {
    pub fn drafted(packet: &Packet, outcome: &'static str) -> Self {
        Self {
            app_id: packet.app_id.clone(),
            outcome,
            tiers: packet.tiers.clone(),
            truncated: packet.truncated,
            local_only: packet.local_only,
            personal: RetrievalCounts::default(),
        }
    }

    /// Adds how many personal records the prompt used. Counts only, never the records.
    pub fn with_personal(mut self, counts: RetrievalCounts) -> Self {
        self.personal = counts;
        self
    }

    pub fn refused(app_id: &str, refusal: &Refusal) -> Self {
        Self {
            app_id: app_id.to_string(),
            outcome: match refusal {
                Refusal::NotChatApp => "refused-not-chat-app",
                Refusal::Blocked(_) => "refused-blocked",
                Refusal::CapLeavesNoContext => "refused-cap",
                Refusal::NotComposer => "refused-not-composer",
                Refusal::EmptyTranscript => "refused-empty",
                Refusal::NoIncomingMessage => "refused-nothing-to-reply-to",
            },
            tiers: Vec::new(),
            truncated: false,
            local_only: false,
            personal: RetrievalCounts::default(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Line;
    use crate::rules::BlockReason;

    #[test]
    fn records_never_contain_bodies_contacts_or_titles() {
        let packet = Packet {
            app_id: "chat.app".into(),
            window_title: "Secret project chat".into(),
            contact: Some("Sam Example".into()),
            lines: vec![Line {
                speaker: "Sam Example".into(),
                from_owner: false,
                text: "the launch code is 1234".into(),
            }],
            tiers: vec![Tier {
                tier: "C2",
                source: "structured",
                chars: 23,
            }],
            truncated: false,
            local_only: true,
        };
        let counts = RetrievalCounts {
            contact: 1,
            profile: 1,
            topics: 2,
            chars: 640,
            ..RetrievalCounts::default()
        };
        let json = AuditRecord::drafted(&packet, "drafted")
            .with_personal(counts)
            .to_json();
        assert!(json.contains(r#""personal":{"contact":1,"profile":1,"style":0,"topics":2,"facts":0,"memories":0,"chars":640}"#), "{json}");
        for secret in ["Secret project", "Sam Example", "launch code", "1234"] {
            assert!(!json.contains(secret), "{secret} leaked into {json}");
        }
        assert!(json.contains("structured") && json.contains("chat.app"));

        let refused =
            AuditRecord::refused("chat.app", &Refusal::Blocked(BlockReason::Rule)).to_json();
        assert!(refused.contains("refused-blocked") && !refused.contains("Sam"));
    }
}
