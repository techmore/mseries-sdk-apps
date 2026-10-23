"""Scoped CSS for the native Messages window.

Colours come from the active elementary stylesheet, so the user's accent
colour and the light/dark preference apply without extra code. A warm ambient
tint distinguishes Messages from Signal. Avatar tones have tested white-initial
contrast. Every selector starts with an ms- class so nothing leaks to other apps.
"""
from messages_model import AVATAR_TONES

# elementary's legacy theme_selected_* pair is not a readable text pair (about
# 1.8:1 in dark mode), so filled surfaces use the accent_bg/accent_fg pair the
# stylesheet designs for exactly this. A declaration that cannot resolve is
# dropped by GTK, so the earlier translucent fallback still applies there.
# _STRONG is a deeper tint of the accent for small marks such as the unread dot.
_STRONG = 'shade(@accent_bg_color, .75)'

STYLE = '''
.ms-window { font-size: 1.12em; background: mix(@theme_bg_color, #dba96b, .10); }
/* Sidebar */
.ms-sidebar { background: @theme_base_color; }
.ms-search { margin: 10px 12px 6px; }
.ms-list { background: transparent; }
.ms-list > row { margin: 1px 8px; border-radius: 10px; }
.ms-row { padding: 8px 6px; }
.ms-contact-name { font-size: 1.4em; font-weight: 600; }
.ms-unread-name { font-weight: 700; }
.ms-row-time { font-size: .82em; opacity: .72; font-feature-settings: "tnum"; }
.ms-preview { font-size: .92em; opacity: .74; }
.ms-unread { min-width: 10px; min-height: 10px; border-radius: 99px; background: alpha(@theme_selected_bg_color,.9); background: ''' + _STRONG + '''; }
.ms-list > row:selected .ms-unread { background: @accent_fg_color; }
.ms-list > row:selected .ms-preview, .ms-list > row:selected .ms-row-time { opacity: .9; }
.ms-section { padding: 10px 14px 4px; font-weight: 600; opacity: .75; }
.ms-pin-note { padding: 2px 14px 8px; }
.ms-pin-row { border-bottom: 1px solid alpha(@theme_fg_color,.08); padding: 12px 0; }
.ms-favorite { padding: 6px 2px; border-radius: 12px; }
.ms-favorite:checked { background: alpha(@theme_selected_bg_color,.18); }
.ms-favorite-name { font-size: 1.1em; font-weight: 600; }
.ms-empty { padding: 36px 24px; opacity: .8; }
.ms-status { padding: 8px 14px; border-top: 1px solid alpha(@theme_fg_color,.10); }
.ms-dot { min-width: 8px; min-height: 8px; border-radius: 99px; background: alpha(@theme_fg_color,.45); }
.ms-dot.ms-ok { background: @success_color; }
.ms-dot.ms-wait { background: @warning_color; }
.ms-dot.ms-off { background: shade(@error_color, .65); }

/* Avatars */
.ms-avatar { border-radius: 99px; color: white; font-weight: 700; }
.ms-avatar-s { font-size: .9em; }
.ms-avatar-m { font-size: 1.05em; }
.ms-avatar-l { font-size: 1.25em; }
.ms-photo { border-radius: 99px; }

/* Conversation */
.ms-heading { padding: 8px 16px; border-bottom: 1px solid alpha(@theme_fg_color,.10); }
.ms-notice { padding: 8px 16px; background: alpha(@theme_selected_bg_color,.12); border-bottom: 1px solid alpha(@theme_selected_bg_color,.2); }
.ms-stamp { font-size: .82em; opacity: .72; margin: 14px 0 6px; }
.ms-sender { font-size: .82em; opacity: .72; margin: 6px 14px 1px; }
.ms-bubble, .ms-chip { border-radius: 18px; padding: 8px 13px; background: alpha(@theme_fg_color,.10); color: @theme_fg_color; }
.ms-outgoing { background: alpha(@theme_selected_bg_color,.35); color: @theme_fg_color; background: @accent_bg_color; color: @accent_fg_color; }
.ms-outgoing link, .ms-outgoing link:visited { color: @accent_fg_color; text-decoration: underline; }
.ms-incoming.ms-last, .ms-incoming.ms-single { border-bottom-left-radius: 5px; }
.ms-outgoing.ms-last, .ms-outgoing.ms-single { border-bottom-right-radius: 5px; }
.ms-sending { opacity: .6; }
.ms-attachment { font-style: italic; margin-top: 3px; }
.ms-reactions { font-size: .95em; padding: 1px 8px; margin: -5px 8px 2px; border-radius: 99px; background: @theme_base_color; border: 1px solid alpha(@theme_fg_color,.18); }
.ms-quote { margin: 6px 16px 0; padding: 4px 6px 4px 10px; border-left: 3px solid @accent_bg_color; background: alpha(@theme_fg_color,.06); border-radius: 6px; font-size: .9em; }
.ms-caption { font-size: .82em; opacity: .72; margin: 2px 4px 0; }
.ms-media { border-radius: 14px; padding: 10px; margin-top: 4px; background: alpha(@theme_fg_color,.06); border: 1px solid alpha(@theme_fg_color,.12); }
.ms-media picture { border-radius: 8px; }
.ms-jump { border-radius: 99px; padding: 5px 14px; margin: 0 0 12px; box-shadow: 0 1px 6px alpha(black,.35); }

/* Composer */
.ms-composer { padding: 10px 16px 6px; border-top: 1px solid alpha(@theme_fg_color,.10); }
.ms-field { border-radius: 20px; border: 1px solid alpha(@theme_fg_color,.22); background: @theme_base_color; padding: 3px 4px 3px 4px; }
.ms-field:focus-within { border-color: alpha(@theme_selected_bg_color,.9); border-color: ''' + _STRONG + '''; box-shadow: 0 0 0 1px alpha(@theme_selected_bg_color,.5); }
.ms-field textview, .ms-field textview text { background: transparent; }
.ms-placeholder { opacity: .6; }
.ms-send { min-width: 30px; min-height: 30px; padding: 0; }
.ms-hint { font-size: .85em; opacity: .75; margin: 0 20px 8px; }
'''


# elementary's stylesheet defines these; GTK's default Adwaita (used by CI and
# other desktops) does not. When they are missing the app defines them from the
# theme's selection pair so every rule above still resolves.
ACCENT_FALLBACKS = {'accent_bg_color': 'theme_selected_bg_color', 'accent_fg_color': 'theme_selected_fg_color'}


def missing_accent_names(context):
    """Accent colour names the current theme does not define."""
    return [name for name in ACCENT_FALLBACKS if not context.lookup_color(name)[0]]


def build_style(define=()):
    """The full stylesheet, with @define-color lines for any names in `define`."""
    defines = ''.join('@define-color %s @%s;\n' % (name, ACCENT_FALLBACKS[name]) for name in define)
    tones = '\n'.join('.ms-tone-%d { background: %s; }' % (i, color) for i, color in enumerate(AVATAR_TONES))
    return defines + STYLE + tones + '\n'
