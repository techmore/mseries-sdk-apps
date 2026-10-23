//! Synthetic chat fixtures (no real messages) for tests and the `mseries-reply` command line.

use crate::context::{ChatSource, FieldDescriptor, Line, Transcript, TranscriptSource, WindowInfo};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub app_id: String,
    pub title: String,
    /// Whether the app carries the chat marker (the command line has no desktop entries to read).
    pub marked: bool,
    pub field: FixtureField,
    pub header: Option<String>,
    pub transcript: Vec<FixtureLine>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureField {
    pub role: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub declared_composer: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureLine {
    pub speaker: String,
    #[serde(default)]
    pub from_owner: bool,
    pub text: String,
}

impl Fixture {
    pub fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|error| format!("invalid fixture: {error}"))
    }
}

impl ChatSource for Fixture {
    fn window(&self) -> WindowInfo {
        WindowInfo {
            app_id: self.app_id.clone(),
            title: self.title.clone(),
            pid: 0,
        }
    }

    fn field(&self) -> FieldDescriptor {
        FieldDescriptor {
            role: self.field.role.clone(),
            label: self.field.label.clone(),
            purpose: self.field.purpose.clone(),
            declared_composer: self.field.declared_composer,
        }
    }

    fn header(&self) -> Option<String> {
        self.header.clone()
    }

    fn transcript(&self, max_lines: usize) -> Transcript {
        let skip = self.transcript.len().saturating_sub(max_lines);
        Transcript {
            source: TranscriptSource::Structured,
            lines: self.transcript[skip..]
                .iter()
                .map(|line| Line {
                    speaker: line.speaker.clone(),
                    from_owner: line.from_owner,
                    text: line.text.clone(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Refusal, build_packet};
    use crate::marker::ChatMarker;
    use crate::rules::{BlockReason, Policy, RULES_VERSION, Rules, Settings};

    fn allow_all() -> Rules {
        Rules::new(Settings {
            version: RULES_VERSION,
            enabled: true,
            policy: Policy::EverywhereExceptBlocked,
            hotkey: None,
            rules: vec![],
        })
    }

    fn marker(fixture: &Fixture) -> impl Fn(&str) -> Option<ChatMarker> + '_ {
        move |_| fixture.marked.then(ChatMarker::default)
    }

    #[test]
    fn the_shipped_fixtures_behave_as_documented() {
        let normal = Fixture::parse(include_str!("../fixtures/lunch.json")).unwrap();
        let packet = build_packet(&normal, &allow_all(), &marker(&normal)).unwrap();
        assert_eq!(packet.lines.len(), 3);

        let injection = Fixture::parse(include_str!("../fixtures/injection.json")).unwrap();
        let packet = build_packet(&injection, &allow_all(), &marker(&injection)).unwrap();
        assert!(packet.lines.last().unwrap().text.contains("IGNORE"));

        let password = Fixture::parse(include_str!("../fixtures/password-field.json")).unwrap();
        assert_eq!(
            build_packet(&password, &allow_all(), &marker(&password)).unwrap_err(),
            Refusal::Blocked(BlockReason::BuiltInField)
        );

        let unmarked = Fixture::parse(include_str!("../fixtures/unmarked-app.json")).unwrap();
        assert_eq!(
            build_packet(&unmarked, &allow_all(), &marker(&unmarked)).unwrap_err(),
            Refusal::NotChatApp
        );
    }
}
