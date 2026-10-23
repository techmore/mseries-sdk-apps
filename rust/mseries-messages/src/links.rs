use percent_encoding::percent_decode_str;
use url::Url;

const BIDI_CONTROLS: &[char] = &[
    '\u{061c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}',
    '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
];
const TRAILING_PUNCTUATION: &[char] = &[
    '.', ',', ';', ':', '!', '?', '\'', '"', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}',
    '\u{2026}',
];
const URL_DELIMITERS: &[char] = &[
    '<', '>', '"', '\u{201c}', '\u{201d}', '\u{2018}', '\u{2019}', '\u{00ab}', '\u{00bb}',
    '\u{2039}', '\u{203a}', '\u{3002}', '\u{ff0c}', '\u{3001}', '\u{ff01}', '\u{ff1f}', '\u{ff1b}',
    '\u{ff1a}',
];

pub fn linkify_message(body: &str) -> String {
    let body = body.replace("\r\n", "\n").replace('\r', "\n");
    if body.chars().any(is_bidi_control) {
        return escape_markup(&body);
    }

    let mut markup = String::new();
    let mut offset = 0;
    let mut search_from = 0;
    while let Some((start, prefix_end)) = find_url_start(&body, search_from) {
        let single_quoted = start > 0 && body[..start].ends_with('\'');
        let end = url_end(&body, prefix_end, single_quoted);
        search_from = end;
        let mut label_end = end;
        while label_end > start {
            let Some(character) = body[..label_end].chars().next_back() else {
                break;
            };
            if !TRAILING_PUNCTUATION.contains(&character) {
                break;
            }
            label_end -= character.len_utf8();
        }
        if label_end <= start {
            continue;
        }
        let label = &body[start..label_end];
        let url = if label
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("www."))
        {
            format!("https://{label}")
        } else {
            label.to_string()
        };
        if !is_safe_web_url(&url) {
            continue;
        }

        markup.push_str(&escape_markup(&body[offset..start]));
        markup.push_str("<a href=\"");
        markup.push_str(&escape_markup(&url));
        markup.push_str("\">");
        markup.push_str(&escape_markup(label));
        markup.push_str("</a>");
        offset = label_end;
    }
    markup.push_str(&escape_markup(&body[offset..]));
    markup
}

fn is_safe_web_url(value: &str) -> bool {
    if !value
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
        && !value
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
    {
        return false;
    }
    let Some((_, remainder)) = value.split_once("://") else {
        return false;
    };
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return false;
    }
    if value
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
        || value.contains('\\')
        || percent_decode_str(value)
            .decode_utf8_lossy()
            .chars()
            .any(is_bidi_control)
    {
        return false;
    }
    let Ok(parsed) = Url::parse(value) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some_and(|host| !host.is_empty())
        && parsed.username().is_empty()
        && parsed.password().is_none()
}

fn find_url_start(body: &str, from: usize) -> Option<(usize, usize)> {
    for (start, _) in body.char_indices().filter(|(index, _)| *index >= from) {
        let suffix = &body[start..];
        let prefix = ["https://", "http://", "www."].into_iter().find(|prefix| {
            suffix
                .get(..prefix.len())
                .is_some_and(|part| part.eq_ignore_ascii_case(prefix))
        });
        let Some(prefix) = prefix else { continue };
        if let Some(previous) = body[..start].chars().next_back()
            && (previous.is_alphanumeric() || previous == '_' || previous == '@' || previous == '/')
        {
            continue;
        }
        return Some((start, start + prefix.len()));
    }
    None
}

fn url_end(body: &str, start: usize, single_quoted: bool) -> usize {
    let mut brackets = Vec::new();
    for (offset, character) in body[start..].char_indices() {
        if character.is_whitespace()
            || character.is_control()
            || URL_DELIMITERS.contains(&character)
            || (single_quoted && character == '\'')
        {
            return start + offset;
        }
        if let Some(closing) = matching_bracket(character) {
            brackets.push(closing);
        } else if is_closing_bracket(character) {
            if brackets.last().copied() != Some(character) {
                return start + offset;
            }
            brackets.pop();
        }
    }
    body.len()
}

fn matching_bracket(character: char) -> Option<char> {
    match character {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '\u{ff08}' => Some('\u{ff09}'),
        '\u{ff3b}' => Some('\u{ff3d}'),
        '\u{ff5b}' => Some('\u{ff5d}'),
        '\u{300c}' => Some('\u{300d}'),
        '\u{300e}' => Some('\u{300f}'),
        _ => None,
    }
}

fn is_closing_bracket(character: char) -> bool {
    matches!(
        character,
        ')' | ']' | '}' | '\u{ff09}' | '\u{ff3d}' | '\u{ff5d}' | '\u{300d}' | '\u{300f}'
    )
}

fn is_bidi_control(character: char) -> bool {
    BIDI_CONTROLS.contains(&character)
}

fn escape_markup(value: &str) -> String {
    gtk::glib::markup_escape_text(value).to_string()
}
