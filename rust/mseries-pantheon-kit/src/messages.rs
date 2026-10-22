use gtk::glib::{self, DateTime, TimeZone};
use serde::Deserialize;
use std::sync::Arc;

/// Format a BlueFerry ISO timestamp in local time using stable calendar labels.
///
/// Relative labels stop at the calendar day so they do not become stale while
/// a conversation is open. Invalid values are reduced to a safe, short line.
pub fn format_message_timestamp(value: &str) -> String {
    let raw = value.trim();
    if raw.is_empty() {
        return String::new();
    }

    let Ok(parsed) = DateTime::from_iso8601(raw, None) else {
        return safe_timestamp_fallback(raw);
    };
    let Ok(local) = parsed.to_timezone(&TimeZone::local()) else {
        return safe_timestamp_fallback(raw);
    };
    let Ok(now) = DateTime::now_local() else {
        return safe_timestamp_fallback(raw);
    };

    const WEEKDAYS: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let days_ago = civil_day_number(now.year(), now.month(), now.day_of_month())
        - civil_day_number(local.year(), local.month(), local.day_of_month());
    let day = match days_ago {
        0 => "Today".to_string(),
        1 => "Yesterday".to_string(),
        2..=6 => WEEKDAYS[(local.day_of_week() - 1) as usize].to_string(),
        _ if local.year() == now.year() => {
            format!(
                "{} {}",
                MONTHS[(local.month() - 1) as usize],
                local.day_of_month()
            )
        }
        _ => format!(
            "{} {}, {}",
            MONTHS[(local.month() - 1) as usize],
            local.day_of_month(),
            local.year()
        ),
    };

    let hour = match local.hour() % 12 {
        0 => 12,
        hour => hour,
    };
    let meridiem = if local.hour() < 12 { "AM" } else { "PM" };
    format!("{day} at {hour}:{:02} {meridiem}", local.minute())
}

fn safe_timestamp_fallback(raw: &str) -> String {
    raw.chars()
        .map(|character| match character {
            '\r' | '\n' => ' ',
            other => other,
        })
        .filter(|character| !character.is_control())
        .take(32)
        .collect()
}

fn civil_day_number(year: i32, month: i32, day: i32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = (year - era * 400) as i64;
    let month = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    i64::from(era) * 146_097 + day_of_era
}

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
    pub reactions: Vec<Reaction>,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub struct Reaction {
    pub reaction: String,
    pub by: String,
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
    #[serde(skip)]
    pub search_index: Arc<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub struct ContactMatch {
    pub name: String,
    pub address: String,
}

impl Thread {
    /// Build a case-folded search index from this bounded conversation snapshot.
    /// Reuse it while the user types instead of normalizing every message body
    /// on each search change.
    pub fn refresh_search_index(&mut self) {
        let preview = self.preview();
        let mut values = Vec::with_capacity(2 + self.recipients.len() + self.messages.len());
        values.push(self.name.as_str());
        values.push(preview.as_str());
        values.extend(self.recipients.iter().map(String::as_str));
        values.extend(self.messages.iter().map(|message| message.body.as_str()));

        let mut search_index = String::new();
        for value in values {
            if !search_index.is_empty() {
                search_index.push('\n');
            }
            search_index.push_str(glib::casefold(value).as_str());
        }
        self.search_index = Arc::new(search_index);
    }

    /// Folds the plain-text Tapbacks exposed by BlueFerry onto the messages
    /// they quote. Unknown or ambiguous targets remain visible as messages.
    pub fn fold_reactions(&mut self) {
        // The Tapback event disappears from the rendered list when it folds;
        // retain its incoming unread state for the conversation indicator.
        self.unread_count = self.unread_count.max(
            self.messages
                .iter()
                .filter(|message| !message.outgoing && !message.read)
                .count(),
        );
        let mut folded: Vec<Message> = Vec::with_capacity(self.messages.len());
        for message in std::mem::take(&mut self.messages) {
            let Some(parsed) = parse_tapback(&message.body) else {
                folded.push(message);
                continue;
            };
            let Some(target) = folded
                .iter_mut()
                .rev()
                .find(|earlier| names_message(&earlier.body, &parsed.quoted))
            else {
                folded.push(message);
                continue;
            };

            let by = if message.outgoing {
                "You".to_string()
            } else if !parsed.who.is_empty() {
                parsed.who
            } else {
                message.sender.clone()
            };
            target.reactions.retain(|reaction| reaction.by != by);
            if !parsed.removed {
                target.reactions.push(Reaction {
                    reaction: parsed.reaction,
                    by,
                });
            }
        }
        self.messages = folded;
    }

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

impl Message {
    /// Returns unique reaction symbols and a readable description for assistive
    /// technology and hover feedback.
    pub fn reaction_summary(&self) -> (String, String) {
        let mut symbols = Vec::new();
        let mut descriptions = Vec::new();
        for reaction in &self.reactions {
            let symbol = reaction_symbol(&reaction.reaction);
            if !symbols.contains(&symbol) {
                symbols.push(symbol.clone());
            }
            let who = if reaction.by.is_empty() {
                "Them"
            } else {
                &reaction.by
            };
            descriptions.push(format!("{symbol} {who}"));
        }
        (symbols.concat(), descriptions.join(" · "))
    }
}

struct ParsedTapback {
    reaction: String,
    quoted: String,
    removed: bool,
    who: String,
}

fn parse_tapback(body: &str) -> Option<ParsedTapback> {
    let text = body.trim();
    let (prefix, quoted) = split_quoted_text(text)?;
    let prefix = prefix.trim_end();
    const REACTIONS: [&str; 6] = [
        "Laughed at",
        "Emphasized",
        "Questioned",
        "Disliked",
        "Loved",
        "Liked",
    ];
    for reaction in REACTIONS {
        if let Some(who) = remove_prefix_suffix(prefix, reaction) {
            return Some(ParsedTapback {
                reaction: reaction.to_string(),
                quoted,
                removed: false,
                who,
            });
        }
    }

    const REMOVED: [(&str, &str); 6] = [
        ("exclamation mark", "Emphasized"),
        ("question mark", "Questioned"),
        ("heart", "Loved"),
        ("dislike", "Disliked"),
        ("laugh", "Laughed at"),
        ("like", "Liked"),
    ];
    for (word, reaction) in REMOVED {
        let suffix = format!("Removed a {word} from");
        if let Some(who) = remove_prefix_suffix(prefix, &suffix) {
            return Some(ParsedTapback {
                reaction: reaction.to_string(),
                quoted,
                removed: true,
                who,
            });
        }
    }

    let parts = prefix.split_whitespace().collect::<Vec<_>>();
    if parts.len() >= 3
        && parts[parts.len() - 3].eq_ignore_ascii_case("Reacted")
        && parts[parts.len() - 1].eq_ignore_ascii_case("to")
    {
        return Some(ParsedTapback {
            reaction: parts[parts.len() - 2].to_string(),
            quoted,
            removed: false,
            who: parts[..parts.len() - 3].join(" "),
        });
    }
    None
}

fn split_quoted_text(text: &str) -> Option<(&str, String)> {
    let (start, opening) = text
        .char_indices()
        .find(|(_, ch)| *ch == '“' || *ch == '"')?;
    let closing = if opening == '“' { '”' } else { '"' };
    let close = text.rfind(closing)?;
    if close <= start || !text[close + closing.len_utf8()..].trim().is_empty() {
        return None;
    }
    let quoted = text[start + opening.len_utf8()..close].to_string();
    if quoted.is_empty() {
        return None;
    }
    Some((&text[..start], quoted))
}

fn remove_prefix_suffix(prefix: &str, suffix: &str) -> Option<String> {
    if prefix.len() < suffix.len() {
        return None;
    }
    let split = prefix.len() - suffix.len();
    if !prefix.get(split..)?.eq_ignore_ascii_case(suffix) {
        return None;
    }
    let before = &prefix[..split];
    if !before.is_empty() && !before.chars().last()?.is_whitespace() {
        return None;
    }
    Some(before.trim().to_string())
}

fn names_message(body: &str, quoted: &str) -> bool {
    let body = squash_whitespace(body);
    let quoted = squash_whitespace(quoted);
    if quoted.is_empty() {
        return false;
    }
    if quoted.ends_with('…') || quoted.ends_with("...") {
        return body.starts_with(quoted.trim_end_matches(['…', '.']).trim_end());
    }
    body == quoted
}

fn squash_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn reaction_symbol(reaction: &str) -> String {
    match reaction {
        "Loved" => "❤️".to_string(),
        "Liked" => "👍".to_string(),
        "Disliked" => "👎".to_string(),
        "Laughed at" => "😂".to_string(),
        "Emphasized" => "‼️".to_string(),
        "Questioned" => "❓".to_string(),
        other => other.to_string(),
    }
}
