"""Use accessibility's editable-field API; never inject keys or read field values."""
import gi
import time
from urllib.parse import urlsplit
gi.require_version('Atspi', '2.0')
from gi.repository import Atspi
from code_offers import code_field


def describe(source, allow_plain=False):
    try:
        deadline = time.monotonic() + .6
        def call(method, *args):
            if time.monotonic() > deadline:
                raise RuntimeError('Field metadata deadline exceeded')
            return method(*args)
        call(source.clear_cache)
        if source.get_role() not in (Atspi.Role.ENTRY, Atspi.Role.TEXT, Atspi.Role.PASSWORD_TEXT):
            return None
        states = source.get_state_set()
        # GTK exposes SENSITIVE for enabled controls; ENABLED is not consistently
        # exported by GTK or Chromium's AT-SPI bridges.
        if not all(states.contains(s) for s in (Atspi.StateType.EDITABLE, Atspi.StateType.SENSITIVE, Atspi.StateType.SHOWING)) or states.contains(Atspi.StateType.DEFUNCT):
            return None
        name = source.get_name() or ''
        attributes = {str(k)[:80]:str(v)[:512] for k,v in call(source.get_attributes).items()}
        if not code_field(name, attributes, allow_plain):
            return None
        # Inspect only length, never existing text. Passive suggestions do not
        # interrupt a code already being entered; the explicit shortcut may replace it.
        if not allow_plain and call(Atspi.Text.get_character_count, source):
            return None
        app = source.get_application()
        origin = ''
        parent = source
        for _ in range(16):
            if time.monotonic() > deadline:
                return None
            if not parent:
                break
            if parent.get_role() == Atspi.Role.DOCUMENT_WEB:
                origin = Atspi.Document.get_document_attribute_value(parent, 'DocURL') or Atspi.Document.get_document_attribute_value(parent, 'URI') or ''
                parsed = urlsplit(origin)
                origin = f'{parsed.scheme}://{parsed.hostname}' if parsed.scheme in ('http', 'https') and parsed.hostname else ''
                break
            parent = parent.get_parent()
        return dict(source=source, path=source.path, name=name[:100] or 'Verification code', application=(app.get_name() or 'Application')[:80], pid=app.get_process_id(), origin=origin, explicit_field=allow_plain, max_length=attributes.get('maxlength', ''), can_insert='EditableText' in source.get_interfaces())
    except Exception:
        return None


def insert(target, code):
    current = describe(target['source'], target.get('explicit_field', False))
    if not current or any(current[k] != target[k] for k in ('pid', 'path', 'name', 'origin')):
        raise RuntimeError('The verification field changed. Focus it again before inserting.')
    if not current['can_insert']:
        raise RuntimeError('This app does not expose accessible text editing. Copy the code instead.')
    if str(current.get('max_length')) == '1' and len(code) > 1:
        raise RuntimeError('This form uses split code fields. Copy and paste into the first box instead.')
    # GTK can expose a working EditableText interface while rejecting the
    # separate Component.GrabFocus request. Insertion addresses this exact
    # revalidated field, so best-effort focus must not prevent the edit.
    try:
        Atspi.Component.grab_focus(target['source'])
    except Exception:
        pass
    if not Atspi.EditableText.set_text_contents(target['source'], code):
        raise RuntimeError('The app did not accept insertion. Copy the code instead.')
    return True
