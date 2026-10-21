"""Keep GTK tests off the owner's real desktop.

The Messages GUI tests open and close dozens of windows and send key events. Run
against a live session they flash windows over the user's work and can take the
keyboard. This module is imported before GTK, decides whether the display is an
isolated virtual one (what xvfb-run provides) and forces the X11 backend, because
with WAYLAND_DISPLAY set GTK would otherwise pick the live Wayland session even
when xvfb-run is used. It is pure so it can be tested without any display.
"""
import re

# xvfb-run -a starts at :99 and counts up. Real sessions, including XWayland, are
# :0 to :89 in practice, so anything lower is refused rather than risked.
FIRST_ISOLATED_DISPLAY = 90


def check(env):
    """(ok, reason): is DISPLAY an isolated virtual display?"""
    display = env.get('DISPLAY', '')
    if not display:
        return False, 'DISPLAY is not set; run under xvfb-run'
    match = re.match(r'^:(\d+)(?:\.\d+)?$', display)
    if match is None:
        return False, 'DISPLAY %r is not a local virtual display; run under xvfb-run' % display
    if int(match.group(1)) < FIRST_ISOLATED_DISPLAY:
        return False, ('DISPLAY %s looks like a real desktop session. These tests open and close '
                       'windows and send key events, so run them under xvfb-run (it uses :99 or higher)' % display)
    return True, ''


def force_x11(env):
    """Make GTK use the X11 display above and never the live Wayland session."""
    env['GDK_BACKEND'] = 'x11'
    env.pop('WAYLAND_DISPLAY', None)
    return env
