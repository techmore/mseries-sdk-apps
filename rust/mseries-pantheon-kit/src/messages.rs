use gtk::glib::{self, DateTime, TimeZone};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
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

/// Visual grouping for one message in a conversation timeline.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MessagePosition {
    #[default]
    Single,
    First,
    Middle,
    Last,
}

/// Presentation hints for rendering a message without modifying transport data.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MessageLayout {
    pub position: MessagePosition,
    pub date_heading: Option<String>,
    pub show_sender: bool,
    pub timestamp_caption: Option<String>,
}

#[derive(Clone, Copy)]
struct LocalMessageMoment {
    year: i32,
    month: i32,
    day: i32,
    unix: i64,
}

struct PreviousMessage<'a> {
    moment: LocalMessageMoment,
    outgoing: bool,
    sender: &'a str,
}

/// Group nearby messages by direction and sender, matching the Messages client
/// timeline behavior. Invalid timestamps remain independent and receive a safe
/// one-line caption instead of affecting neighboring groups.
pub fn layout_messages<'a>(
    messages: impl IntoIterator<Item = &'a Message>,
    is_group: bool,
) -> Vec<MessageLayout> {
    let messages = messages.into_iter().collect::<Vec<_>>();
    let mut layout = vec![MessageLayout::default(); messages.len()];
    let mut run = Vec::new();
    let mut previous: Option<PreviousMessage<'_>> = None;

    for (index, message) in messages.into_iter().enumerate() {
        let Some(moment) = local_message_moment(&message.timestamp) else {
            close_message_run(&mut layout, &mut run);
            layout[index].show_sender = is_group && !message.outgoing && !message.sender.is_empty();
            let caption = format_message_timestamp(&message.timestamp);
            layout[index].timestamp_caption = (!caption.is_empty()).then_some(caption);
            previous = None;
            continue;
        };

        let sender = if is_group && !message.outgoing {
            message.sender.as_str()
        } else {
            ""
        };
        let gap = previous
            .as_ref()
            .map(|prior| moment.unix - prior.moment.unix);
        let needs_heading = previous.as_ref().is_none_or(|prior| {
            (moment.year, moment.month, moment.day)
                != (prior.moment.year, prior.moment.month, prior.moment.day)
                || gap.is_some_and(|seconds| seconds >= 30 * 60)
        });

        if needs_heading {
            close_message_run(&mut layout, &mut run);
            let heading = format_message_timestamp(&message.timestamp);
            if !heading.is_empty() {
                layout[index].date_heading = Some(heading);
            }
        } else if previous.as_ref().is_some_and(|prior| {
            message.outgoing != prior.outgoing
                || sender != prior.sender
                || gap.is_some_and(|seconds| seconds >= 5 * 60)
        }) {
            close_message_run(&mut layout, &mut run);
        }

        if run.is_empty() {
            layout[index].show_sender = is_group && !message.outgoing && !message.sender.is_empty();
        }
        run.push(index);
        previous = Some(PreviousMessage {
            moment,
            outgoing: message.outgoing,
            sender,
        });
    }

    close_message_run(&mut layout, &mut run);
    layout
}

fn local_message_moment(value: &str) -> Option<LocalMessageMoment> {
    let parsed = DateTime::from_iso8601(value.trim(), None).ok()?;
    let local = parsed.to_timezone(&TimeZone::local()).ok()?;
    Some(LocalMessageMoment {
        year: local.year(),
        month: local.month(),
        day: local.day_of_month(),
        unix: local.to_unix(),
    })
}

fn close_message_run(layout: &mut [MessageLayout], run: &mut Vec<usize>) {
    let position = match run.len() {
        0 => return,
        1 => MessagePosition::Single,
        _ => MessagePosition::Last,
    };
    if run.len() > 1 {
        for index in &run[..run.len() - 1] {
            layout[*index].position = if *index == run[0] {
                MessagePosition::First
            } else {
                MessagePosition::Middle
            };
        }
    }
    if let Some(index) = run.last() {
        layout[*index].position = position;
    }
    run.clear();
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

/// One bounded entry from BlueFerry's supported `Messages1.ListRecent` API.
/// Fields introduced by newer BlueFerry builds remain optional for API v2
/// compatibility.
#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub struct RecentMessage {
    pub handle: String,
    pub sender: String,
    pub sender_phone_norm: String,
    pub contact_name: String,
    pub sender_alphanumeric: Option<String>,
    pub body: String,
    pub timestamp: String,
    pub read: bool,
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

#[cfg(test)]
mod recent_inbox_tests {
    use super::*;

    fn direct(key: &str, recipient: &str) -> Thread {
        Thread {
            key: key.to_string(),
            name: "Mike".to_string(),
            recipients: vec![recipient.to_string()],
            reply_ready: true,
            ..Thread::default()
        }
    }

    fn recent(handle: &str, sender: &str, timestamp: &str, read: bool) -> RecentMessage {
        RecentMessage {
            handle: handle.to_string(),
            sender: sender.to_string(),
            sender_phone_norm: sender.trim_start_matches('+').to_string(),
            body: handle.to_string(),
            timestamp: timestamp.to_string(),
            read,
            ..RecentMessage::default()
        }
    }

    #[test]
    fn inbox_snapshot_matches_phone_formatting_and_counts_unread_messages() {
        let thread = direct("email:mike@example.com", "(908) 642-0729");
        let merged = merge_recent_inbox(
            vec![thread],
            &[
                recent("new1", "+19086420729", "2026-10-20T20:18:09", false),
                recent("new2", "19086420729", "2026-10-20T20:32:02", false),
            ],
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].key, "email:mike@example.com");
        assert_eq!(
            merged[0]
                .messages
                .iter()
                .map(|message| message.handle.as_str())
                .collect::<Vec<_>>(),
            ["new1", "new2"]
        );
        assert_eq!(merged[0].unread(), 2);
        assert!(merged[0].messages.iter().all(|message| message.phone_list));
    }

    #[test]
    fn ambiguous_direct_owners_get_one_phone_only_thread() {
        let merged = merge_recent_inbox(
            vec![direct("a", "+19086420729"), direct("b", "9086420729")],
            &[
                recent("new1", "19086420729", "2026-10-20T20:18:09", true),
                recent("new2", "+19086420729", "2026-10-20T20:32:02", true),
            ],
        );
        assert!(
            merged
                .iter()
                .all(|thread| thread.messages.is_empty() || thread.inbox_only)
        );
        let phone_only = merged.iter().find(|thread| thread.inbox_only).unwrap();
        assert_eq!(phone_only.messages.len(), 2);
        assert_eq!(phone_only.key, "inbox:19086420729");
    }

    #[test]
    fn groups_invalid_sender_fallbacks_and_duplicate_handles_are_safe() {
        let group = Thread {
            key: "group".to_string(),
            is_group: true,
            recipients: vec!["+19086420729".to_string(), "+19086420730".to_string()],
            ..Thread::default()
        };
        let initial = merge_recent_inbox(
            vec![group],
            &[
                recent("same", "19086420729", "2026-10-20T20:18:09", true),
                recent("bad", "(908) 642-0729", "2026-10-20T20:19:09", true),
            ],
        );
        assert_eq!(
            initial.len(),
            2,
            "the group plus one direct phone-only thread"
        );
        let merged = merge_recent_inbox(
            initial,
            &[recent("same", "19086420729", "2026-10-20T20:18:09", true)],
        );
        let phone_only = merged.iter().find(|thread| thread.inbox_only).unwrap();
        assert_eq!(
            phone_only.messages.len(),
            1,
            "a repeated handle must not duplicate"
        );
        assert!(
            merged
                .iter()
                .find(|thread| thread.key == "group")
                .unwrap()
                .messages
                .is_empty()
        );
    }

    #[test]
    fn alphanumeric_sender_metadata_disables_replying_to_the_phone_only_route() {
        let mut item = recent("alert", "12345", "2026-10-20T20:18:09", false);
        item.sender_alphanumeric = Some("Bank Alert".to_string());
        let merged = merge_recent_inbox(Vec::new(), &[item]);
        let thread = merged.first().unwrap();
        assert_eq!(thread.name, "Bank Alert");
        assert!(!thread.reply_ready);
        assert_eq!(thread.sender_alphanumeric.as_deref(), Some("Bank Alert"));
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
    pub inbox_only: bool,
    #[serde(skip)]
    pub sender_alphanumeric: Option<String>,
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

/// Merge the phone's bounded inbox snapshot into backend threads without
/// guessing a group route or choosing between ambiguous direct conversations.
/// Messages unmatched to one direct conversation get a phone-only local thread.
pub fn merge_recent_inbox(mut threads: Vec<Thread>, recent: &[RecentMessage]) -> Vec<Thread> {
    let mut handles = threads
        .iter()
        .flat_map(|thread| thread.messages.iter())
        .filter_map(|message| (!message.handle.is_empty()).then_some(message.handle.clone()))
        .collect::<HashSet<_>>();
    let mut owners: HashMap<String, Vec<usize>> = HashMap::new();
    let mut phone_only: HashMap<String, usize> = HashMap::new();

    for (index, thread) in threads.iter().enumerate() {
        if thread.inbox_only
            && let Some(recipient) = thread.recipients.first()
            && let Some(address) = normalize_thread_recipient(recipient)
        {
            for identity in inbox_identity_keys(&address) {
                phone_only.entry(identity).or_insert(index);
            }
        }
        if thread.inbox_only || thread.is_group || thread.recipients.len() != 1 {
            continue;
        }
        let Some(address) = normalize_thread_recipient(&thread.recipients[0]) else {
            continue;
        };
        for identity in inbox_identity_keys(&address) {
            let candidates = owners.entry(identity).or_default();
            if !candidates.contains(&index) {
                candidates.push(index);
            }
        }
    }

    for item in recent {
        // Prefer BlueFerry's normalized address. It is generated from the
        // phone's sender field and avoids interpreting arbitrary display text.
        let sender = if item.sender_phone_norm.trim().is_empty() {
            item.sender.as_str()
        } else {
            item.sender_phone_norm.as_str()
        };
        let Some(address) = normalize_inbox_address(sender) else {
            continue;
        };
        let identities = inbox_identity_keys(&address);
        let mut candidates = Vec::new();
        for identity in &identities {
            if let Some(indices) = owners.get(identity) {
                for index in indices {
                    if !candidates.contains(index) {
                        candidates.push(*index);
                    }
                }
            }
        }
        let mut target = if candidates.len() == 1 {
            Some(candidates[0])
        } else {
            identities
                .iter()
                .find_map(|identity| phone_only.get(identity).copied())
        };

        if !item.handle.is_empty() && handles.contains(&item.handle) {
            continue;
        }
        if target.is_none() {
            let index = threads.len();
            let mut thread = Thread {
                key: format!("inbox:{address}"),
                name: if item.contact_name.trim().is_empty() {
                    if item.sender.trim().is_empty() {
                        address.clone()
                    } else {
                        item.sender.clone()
                    }
                } else {
                    item.contact_name.clone()
                },
                recipients: vec![address.clone()],
                reply_ready: true,
                local_only: true,
                inbox_only: true,
                ..Thread::default()
            };
            if let Some(sender) = item
                .sender_alphanumeric
                .as_deref()
                .filter(|s| !s.is_empty())
            {
                thread.name = sender.to_string();
                thread.reply_ready = false;
                thread.sender_alphanumeric = Some(sender.to_string());
            }
            threads.push(thread);
            for identity in identities {
                phone_only.entry(identity).or_insert(index);
            }
            target = Some(index);
        }

        let Some(index) = target else {
            continue;
        };
        let thread = &mut threads[index];
        if let Some(sender) = item
            .sender_alphanumeric
            .as_deref()
            .filter(|s| !s.is_empty())
        {
            thread.name = sender.to_string();
            thread.reply_ready = false;
            thread.sender_alphanumeric = Some(sender.to_string());
        }
        thread.messages.push(Message {
            handle: item.handle.clone(),
            body: item.body.clone(),
            timestamp: item.timestamp.clone(),
            outgoing: false,
            read: item.read,
            sender: item.sender.clone(),
            phone_list: true,
            ..Message::default()
        });
        if !item.handle.is_empty() {
            handles.insert(item.handle.clone());
        }
    }

    for thread in &mut threads {
        thread
            .messages
            .sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
        if let Some(last) = thread.messages.last() {
            thread.last_ts.clone_from(&last.timestamp);
        }
        thread.refresh_search_index();
    }
    threads.sort_by(|left, right| right.last_ts.cmp(&left.last_ts));
    threads
}

fn inbox_identity_keys(address: &str) -> Vec<String> {
    if address.contains('@') {
        return vec![format!("email:{}", address.to_ascii_lowercase())];
    }
    let digits = address
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(char::from)
        .collect::<String>();
    if digits.is_empty() {
        return Vec::new();
    }
    let mut keys = vec![format!("phone:{digits}")];
    if digits.len() == 10 {
        keys.push(format!("phone:1{digits}"));
    } else if digits.len() == 11 && digits.starts_with('1') {
        keys.push(format!("phone:{}", &digits[1..]));
    }
    keys
}

/// Whether two valid direct-address spellings resolve to the same conservative
/// inbox identity, including North American ten/eleven-digit number twins.
pub fn recipients_share_inbox_identity(left: &str, right: &str) -> bool {
    let (Some(left), Some(right)) = (
        normalize_thread_recipient(left),
        normalize_thread_recipient(right),
    ) else {
        return false;
    };
    let left_keys = inbox_identity_keys(&left);
    let right_keys = inbox_identity_keys(&right);
    left_keys.iter().any(|key| right_keys.contains(key))
}

fn normalize_thread_recipient(input: &str) -> Option<String> {
    let candidate = input.trim();
    let bytes = candidate.as_bytes();
    let phone = !bytes.is_empty()
        && (bytes[0] != b'+' || bytes.len() > 1)
        && bytes.iter().enumerate().all(|(index, byte)| {
            byte.is_ascii_digit() || (index == 0 && *byte == b'+') || b" ()-.".contains(byte)
        });
    if phone {
        let digits = candidate
            .bytes()
            .filter(u8::is_ascii_digit)
            .map(char::from)
            .collect::<String>();
        if (3..=20).contains(&digits.len()) {
            return Some(format!(
                "{}{digits}",
                if candidate.starts_with('+') { "+" } else { "" }
            ));
        }
        return None;
    }
    normalize_inbox_address(candidate)
}

/// Inbox identities accept conservative phone spelling and email addresses.
/// Parentheses are excluded for raw sender fallbacks; BlueFerry's normalized
/// sender field already handles legitimate formatted phone numbers.
fn normalize_inbox_address(input: &str) -> Option<String> {
    let candidate = input.trim();
    let bytes = candidate.as_bytes();
    let phone = !bytes.is_empty()
        && (bytes[0] != b'+' || bytes.len() > 1)
        && bytes.iter().enumerate().all(|(index, byte)| {
            byte.is_ascii_digit() || (index == 0 && *byte == b'+') || b" -.".contains(byte)
        });
    if phone {
        let digits = candidate
            .bytes()
            .filter(u8::is_ascii_digit)
            .map(char::from)
            .collect::<String>();
        if (3..=20).contains(&digits.len()) {
            return Some(format!(
                "{}{digits}",
                if candidate.starts_with('+') { "+" } else { "" }
            ));
        }
        return None;
    }

    if candidate.len() > 254 || candidate.matches('@').count() != 1 || !candidate.is_ascii() {
        return None;
    }
    let (local, domain) = candidate.split_once('@')?;
    if local.is_empty()
        || local.len() > 64
        || domain.is_empty()
        || domain.len() > 253
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !local
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+/=?^_`{|}~.-".contains(&byte))
    {
        return None;
    }
    for label in domain.split('.') {
        if label.is_empty()
            || label.len() > 63
            || !label.as_bytes()[0].is_ascii_alphanumeric()
            || !label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return None;
        }
    }
    Some(format!("{local}@{}", domain.to_ascii_lowercase()))
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
