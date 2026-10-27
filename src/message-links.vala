namespace SignalNative {
    // Detects web links in message text and renders them as safe Pango markup.
    public class MessageLinks : Object {
        private static Regex? link_pattern = null;

        // Bidirectional formatting characters can disguise where a link really goes.
        public static bool has_direction_control (string text) {
            int index = 0;
            unichar c;
            while (text.get_next_char (ref index, out c)) {
                if ((c >= 0x202A && c <= 0x202E) || (c >= 0x2066 && c <= 0x2069) || c == 0x200E || c == 0x200F || c == 0x061C) {
                    return true;
                }
            }
            return false;
        }

        // Trailing punctuation belongs to the sentence, not to the link; an unmatched
        // closing bracket is dropped but a balanced one (wikipedia.org/A_(b)) is kept.
        public static string trim_trailing (string link) {
            string result = link;
            while (result.length > 0) {
                char last = result[result.length - 1];
                if (last == '.' || last == ',' || last == ';' || last == ':' || last == '!' || last == '?' ||
                    last == '\'' || last == '"') {
                    result = result.substring (0, result.length - 1);
                } else if (last == ')' || last == ']' || last == '}') {
                    char open = last == ')' ? '(' : last == ']' ? '[' : '{';
                    int opened = 0, closed = 0;
                    for (int i = 0; i < result.length; i++) {
                        if (result[i] == open) { opened++; }
                        if (result[i] == last) { closed++; }
                    }
                    if (closed > opened) { result = result.substring (0, result.length - 1); } else { break; }
                } else {
                    break;
                }
            }
            return result;
        }

        // Only http(s) links with a real host and no hidden direction controls become clickable.
        public static string? safe_link (string candidate) {
            string link = trim_trailing (candidate);
            if (link == "" || has_direction_control (link)) { return null; }
            try {
                var uri = Uri.parse (link, UriFlags.NONE);
                string scheme = uri.get_scheme ();
                if (scheme != "http" && scheme != "https") { return null; }
                string? host = uri.get_host ();
                if (host == null || host == "") { return null; }
                if (uri.get_user () != null) { return null; }
                return link;
            } catch (Error error) {
                return null;
            }
        }

        public static string[] find_links (string text) {
            string[] found = {};
            if (link_pattern == null) {
                try {
                    link_pattern = new Regex ("https?://[^\\s<>\"\\x{0}-\\x{1f}\\x{7f}]+", RegexCompileFlags.OPTIMIZE);
                } catch (Error error) {
                    return found;
                }
            }
            MatchInfo match;
            if (link_pattern.match (text, 0, out match)) {
                try {
                    do {
                        string? safe = safe_link (match.fetch (0));
                        if (safe != null) { found += safe; }
                    } while (match.next ());
                } catch (Error error) {}
            }
            return found;
        }

        // Escapes the text and wraps each safe link in an anchor.
        public static string markup (string text) {
            var output = new StringBuilder ();
            int cursor = 0;
            if (link_pattern == null) { find_links (""); }
            MatchInfo? match = null;
            if (link_pattern != null && link_pattern.match (text, 0, out match)) {
                try {
                    do {
                        int start, end;
                        if (!match.fetch_pos (0, out start, out end)) { continue; }
                        string candidate = text.substring (start, end - start);
                        string? safe = safe_link (candidate);
                        if (safe == null || start < cursor) { continue; }
                        int link_end = start + safe.length;
                        output.append (Markup.escape_text (text.substring (cursor, start - cursor)));
                        output.append ("<a href=\"%s\">%s</a>".printf (Markup.escape_text (safe), Markup.escape_text (safe)));
                        cursor = link_end;
                    } while (match.next ());
                } catch (Error error) {}
            }
            output.append (Markup.escape_text (text.substring (cursor)));
            return output.str;
        }
    }
}
