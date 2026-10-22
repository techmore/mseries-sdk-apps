"""Local favorites and ordered Apple pin snapshots live in the desktop keyring."""
from copy import deepcopy
from datetime import datetime, timezone
import hashlib
import json
import re
from pathlib import Path
from messages_model import address_key

MAX_BYTES = 131072
MAX_PINS = 32
MAX_LOCAL_PINS = 64
SCHEMA = 'io.elementary.mseries.apple-message-pins'


def validate_snapshot(value):
    if not isinstance(value, dict) or value.get('schema') != SCHEMA or value.get('version') != 1 or value.get('source') not in ('macos-imcore', 'macos-pin-cache'):
        raise ValueError('This is not a supported Apple Messages pin export.')
    try:
        stamp = datetime.fromisoformat(value['captured_at'].replace('Z', '+00:00'))
        if stamp.tzinfo is None or stamp.timestamp() > datetime.now(timezone.utc).timestamp() + 300:
            raise ValueError()
    except (KeyError, TypeError, AttributeError, ValueError, OverflowError):
        raise ValueError('The pin export has an invalid capture time.') from None
    entries = value.get('pins')
    if not isinstance(entries, list) or len(entries) > MAX_PINS:
        raise ValueError('The pin export has an invalid conversation list.')
    seen, pins = set(), []
    for entry in entries:
        identifier = entry.get('apple_identifier') if isinstance(entry, dict) else None
        if not isinstance(identifier, str) or not 0 < len(identifier) <= 2048 or re.search(r'[\x00-\x1f\x7f]', identifier) or identifier in seen:
            raise ValueError('The pin export has invalid or duplicate identifiers.')
        seen.add(identifier)
        pins.append({'apple_identifier': identifier})
    # Do not persist arbitrary extra metadata from an imported document. Keep
    # opaque identifiers so unresolved groups can be explicitly matched later.
    result = dict(schema=SCHEMA, version=1, source=value['source'], captured_at=stamp.isoformat(), pins=pins)
    if value.get('source_modified_at'):
        try:
            modified = datetime.fromisoformat(value['source_modified_at'].replace('Z', '+00:00'))
            if modified.tzinfo is None or modified.timestamp() > stamp.timestamp() + 300:
                raise ValueError()
        except (ValueError, AttributeError, TypeError):
            raise ValueError('The Apple pin modification time is invalid.') from None
        result['source_modified_at'] = modified.isoformat()
    return result


def read_snapshot(path):
    with Path(path).open('rb') as stream:
        data = stream.read(MAX_BYTES + 1)
    if len(data) > MAX_BYTES:
        raise ValueError('The pin export is too large.')
    try:
        value = json.loads(data)
    except (ValueError, UnicodeError):
        raise ValueError('The pin export could not be read.') from None
    return validate_snapshot(value)


def suggested_matches(snapshot, threads, previous=None):
    """Only exact, unique individual addresses. Group names are never identity."""
    result = []
    known = {}
    if previous and previous.get('snapshot'):
        known = {pin['apple_identifier']: key for pin, key in zip(previous['snapshot']['pins'], previous['choices']) if key in threads}
    for pin in snapshot['pins']:
        if pin['apple_identifier'] in known:
            result.append(known[pin['apple_identifier']])
            continue
        address = address_key(pin['apple_identifier'])
        matches = [t['key'] for t in threads.values() if address and not t.get('is_group') and len(t.get('recipients', [])) == 1 and address_key(t['recipients'][0]) == address]
        result.append(matches[0] if len(matches) == 1 else None)
    return result


def default_state():
    return dict(version=2, snapshot=None, choices=[], local_pins=[], hidden_imports=[])


def validate_state(value):
    """Load the bounded local state while retaining v1 Apple-pin imports."""
    if not isinstance(value, dict) or value.get('version') not in (1, 2):
        raise ValueError('The saved pin import is unsupported.')
    snapshot = validate_snapshot(value['snapshot']) if value.get('snapshot') is not None else None
    choices = value.get('choices', [])
    if not isinstance(choices, list) or (snapshot is None and choices):
        raise ValueError('The saved conversation matches are invalid.')
    if snapshot and len(choices) != len(snapshot['pins']):
        raise ValueError('The saved conversation matches are invalid.')
    if any(key is not None and (not isinstance(key, str) or not 0 < len(key) <= 4096) for key in choices):
        raise ValueError('The saved conversation matches are invalid.')
    if len(set(key for key in choices if key)) != len([key for key in choices if key]):
        raise ValueError('The saved conversation matches contain duplicates.')

    local_pins = value.get('local_pins', []) if value.get('version') == 2 else []
    if not isinstance(local_pins, list) or len(local_pins) > MAX_LOCAL_PINS:
        raise ValueError('The saved local favorites are invalid.')
    if any(not isinstance(key, str) or not 0 < len(key) <= 4096 or
           not key.startswith(('direct:', 'thread:')) or
           key.endswith(':') for key in local_pins):
        raise ValueError('The saved local favorites are invalid.')
    if len(set(local_pins)) != len(local_pins):
        raise ValueError('The saved local favorites contain duplicates.')

    hidden_imports = value.get('hidden_imports', []) if value.get('version') == 2 else []
    if not isinstance(hidden_imports, list) or len(hidden_imports) > MAX_PINS:
        raise ValueError('The saved local pin overrides are invalid.')
    if any(not isinstance(identifier, str) or not 0 < len(identifier) <= 2048 or
           re.search(r'[\x00-\x1f\x7f]', identifier) for identifier in hidden_imports):
        raise ValueError('The saved local pin overrides are invalid.')
    if len(set(hidden_imports)) != len(hidden_imports):
        raise ValueError('The saved local pin overrides contain duplicates.')
    known_ids = {pin['apple_identifier'] for pin in snapshot['pins']} if snapshot else set()
    if any(identifier not in known_ids for identifier in hidden_imports):
        raise ValueError('The saved local pin overrides do not match the imported snapshot.')
    return dict(version=2, snapshot=snapshot, choices=list(choices),
                local_pins=list(local_pins), hidden_imports=list(hidden_imports))


def matched_state(snapshot, choices, threads, previous=None):
    snapshot = validate_snapshot(snapshot)
    if len(choices) != len(snapshot['pins']):
        raise ValueError('Review every imported pin before continuing.')
    nonempty = [key for key in choices if key]
    if len(set(nonempty)) != len(nonempty) or any(key not in threads for key in nonempty):
        raise ValueError('Choose each conversation once. A conversation may have changed; review the matches.')
    prior = validate_state(previous) if previous is not None else default_state()
    imported_identities = {local_identity(threads[key]) for key in nonempty}
    local_pins = [identity for identity in prior['local_pins'] if identity not in imported_identities]
    previous_choices = dict(zip(
        (pin['apple_identifier'] for pin in (prior['snapshot'] or {}).get('pins', [])),
        prior['choices']))
    hidden_imports = [identifier for identifier, choice in zip(
        (pin['apple_identifier'] for pin in snapshot['pins']), choices)
        if identifier in prior['hidden_imports'] and previous_choices.get(identifier) == choice]
    return dict(version=2, snapshot=snapshot, choices=list(choices),
                local_pins=local_pins, hidden_imports=hidden_imports)


def effective_choices(threads, state):
    choices = list(state.get('choices', []))
    if state.get('snapshot'):
        suggested = suggested_matches(state['snapshot'], threads)
        for index, key in enumerate(choices):
            # Only phone-only/new individual identities get promoted. Never
            # replace a missing group or an unresolved explicit choice by name.
            if key and key not in threads and key.startswith(('inbox:', 'new:')):
                choices[index] = suggested[index]
    return choices


def ordered_keys(threads, state):
    selected = effective_pins(threads, state)
    ranks = {key: len(selected) - index for index, key in enumerate(selected)}
    return sorted(threads, key=lambda key: (ranks.get(key, 0), threads[key].get('last_ts', '')), reverse=True)


def effective_pins(threads, state):
    """Imported Mac pins first, followed by this computer's local favorites."""
    snapshot = state.get('snapshot')
    hidden = set(state.get('hidden_imports', []))
    pinned = []
    if snapshot:
        for pin, key in zip(snapshot['pins'], effective_choices(threads, state)):
            if pin['apple_identifier'] not in hidden and key in threads and key not in pinned:
                pinned.append(key)
    for identity in state.get('local_pins', []):
        key = resolve_local_identity(identity, threads)
        if key and key not in pinned:
            pinned.append(key)
    return pinned


def local_identity(thread):
    """Stable local key: direct chats follow the unique phone address, groups their backend ID."""
    recipients = thread.get('recipients', [])
    if not thread.get('is_group') and len(recipients) == 1:
        address = address_key(recipients[0])
        if address:
            return 'direct:' + address
    key = thread.get('key')
    if not isinstance(key, str) or not key:
        raise ValueError('This conversation has no stable identity yet.')
    return 'thread:' + key


def resolve_local_identity(identity, threads):
    if identity.startswith('thread:'):
        key = identity[len('thread:'):]
        return key if key in threads else None
    if identity.startswith('direct:'):
        address = identity[len('direct:'):]
        matches = [key for key, thread in threads.items()
                   if not thread.get('is_group') and len(thread.get('recipients', [])) == 1
                   and address_key(thread['recipients'][0]) == address]
        return matches[0] if len(matches) == 1 else None
    # v2 was introduced with this format. Refuse to guess from an old opaque ID.
    return None


def unresolved_imports(state, threads):
    """Count visible imported pins that still need a local conversation match."""
    snapshot = state.get('snapshot')
    if not snapshot:
        return 0
    hidden = set(state.get('hidden_imports', []))
    choices = effective_choices(threads, state)
    return sum(pin['apple_identifier'] not in hidden and key not in threads
               for pin, key in zip(snapshot['pins'], choices))


def toggle_pin_state(key, threads, state):
    """Toggle a favorite locally without changing the imported Apple snapshot."""
    if key not in threads or threads[key].get('new'):
        raise ValueError('This conversation cannot be saved as a favorite yet.')
    result = validate_state(state)
    identity = local_identity(threads[key])
    matching = []
    if result['snapshot']:
        matching = [(pin['apple_identifier'], choice)
                    for pin, choice in zip(result['snapshot']['pins'], effective_choices(threads, result))
                    if choice == key]
    hidden = set(result['hidden_imports'])
    was_pinned = key in effective_pins(threads, result)
    if identity in result['local_pins']:
        result['local_pins'].remove(identity)
        # If the same chat is both locally pinned and imported, remove both
        # visible sources in one action while keeping the Mac snapshot intact.
        visible_imports = [identifier for identifier, _choice in matching if identifier not in hidden]
        if was_pinned and visible_imports:
            result['hidden_imports'] = list(dict.fromkeys([*result['hidden_imports'], *visible_imports]))
        return result

    if matching:
        visible = [identifier for identifier, _choice in matching if identifier not in hidden]
        if was_pinned and visible:
            result['hidden_imports'] = list(dict.fromkeys([*result['hidden_imports'], *visible]))
        else:
            result['hidden_imports'] = [identifier for identifier in result['hidden_imports']
                                        if identifier not in {identifier for identifier, _choice in matching}]
        return result

    if len(result['local_pins']) >= MAX_LOCAL_PINS:
        raise ValueError('You can save up to 64 favorite conversations.')
    result['local_pins'].append(identity)
    return result


def profile_key():
    from blueferry import config
    phone = config.read_local_env().get('BLUEFERRY_MAC', '')
    if not config.is_valid_mac(phone):
        raise ValueError('Connect an iPhone before importing its pins.')
    return hashlib.sha256(phone.upper().encode()).hexdigest()


class PinStore:
    """Use a separate app keyring item for order and import provenance.

    BlueFerry's star store is an unordered set; modifying it would lose Apple's
    order and could overwrite preferences in other BlueFerry clients.
    """
    def __init__(self, profile):
        import gi
        gi.require_version('Secret', '1')
        from gi.repository import Secret
        self.secret = Secret
        self.schema = Secret.Schema.new('io.elementary.mseries.Messages.ApplePins', Secret.SchemaFlags.NONE,
                                        {'phone': Secret.SchemaAttributeType.STRING})
        self.attributes = {'phone': profile}

    def load(self):
        text = self.secret.password_lookup_sync(self.schema, self.attributes, None)
        if not text:
            return default_state()
        if len(text.encode()) > MAX_BYTES:
            raise ValueError('The saved pin import is too large.')
        return validate_state(json.loads(text))

    def save(self, state):
        state = validate_state(state)
        text = json.dumps(state, ensure_ascii=True)
        if len(text.encode()) > MAX_BYTES:
            raise ValueError('The saved pin import is too large.')
        if not self.secret.password_store_sync(self.schema, self.attributes, self.secret.COLLECTION_DEFAULT,
                                               'Messages — favorites and imported Apple pins', text, None):
            raise RuntimeError('The desktop keyring did not save the imported pins.')
        return deepcopy(state)
