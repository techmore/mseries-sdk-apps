"""Ordered Apple pin snapshots. Private state lives in the desktop keyring."""
from copy import deepcopy
from datetime import datetime, timezone
import hashlib
import json
import re
from pathlib import Path
from messages_model import address_key

MAX_BYTES = 131072
MAX_PINS = 32
SCHEMA = 'io.elementary.mseries.apple-message-pins'
KEYRING_SCHEMA = 'org.freedesktop.Secret.Generic'
KEYRING_APPLICATION = 'io.elementary.mseries.Messages'
LEGACY_KEYRING_SCHEMA = 'io.elementary.mseries.Messages.ApplePins'


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


def matched_state(snapshot, choices, threads):
    snapshot = validate_snapshot(snapshot)
    if len(choices) != len(snapshot['pins']):
        raise ValueError('Review every imported pin before continuing.')
    nonempty = [key for key in choices if key]
    if len(set(nonempty)) != len(nonempty) or any(key not in threads for key in nonempty):
        raise ValueError('Choose each conversation once. A conversation may have changed; review the matches.')
    return dict(version=1, snapshot=snapshot, choices=list(choices))


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
    selected = [key for key in effective_choices(threads, state) if key in threads]
    ranks = {key: len(selected) - index for index, key in enumerate(selected)}
    return sorted(threads, key=lambda key: (ranks.get(key, 0), threads[key].get('last_ts', '')), reverse=True)


def profile_key(status):
    """Return BlueFerry's stable phone identity; never derive a new key from MAC."""
    profile = status.get('phone_profile_id') if isinstance(status, dict) else None
    if isinstance(profile, str) and re.fullmatch(
            r'[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}', profile):
        return profile
    raise ValueError('Update BlueFerry to enable phone-scoped imported pins.')


def _legacy_profile_key():
    """Old MAC-derived key, used only to migrate an existing private keyring item."""
    try:
        from blueferry import config
        phone = config.read_local_env().get('BLUEFERRY_MAC', '')
        if not config.is_valid_mac(phone):
            return None
    except (ImportError, OSError, ValueError):
        return None
    return hashlib.sha256(phone.upper().encode()).hexdigest()


class PinStore:
    """Use a separate app keyring item for order and import provenance.

    BlueFerry's star store is an unordered set; modifying it would lose Apple's
    order and could overwrite preferences in other BlueFerry clients.
    """
    def __init__(self, profile, legacy_profile=None):
        import gi
        gi.require_version('Secret', '1')
        from gi.repository import Secret
        self.secret = Secret
        # Match Rust's `secret-tool` records so either Messages client reads
        # the same phone-scoped preferences.
        self.schema = Secret.Schema.new(KEYRING_SCHEMA, Secret.SchemaFlags.NONE,
                                        {'application': Secret.SchemaAttributeType.STRING,
                                         'phone': Secret.SchemaAttributeType.STRING})
        self.attributes = {'application': KEYRING_APPLICATION, 'phone': profile}
        self.legacy_schema = Secret.Schema.new(LEGACY_KEYRING_SCHEMA, Secret.SchemaFlags.NONE,
                                               {'phone': Secret.SchemaAttributeType.STRING})
        self.legacy_attributes = {'phone': legacy_profile} if legacy_profile else None

    def _decode(self, text):
        if len(text.encode()) > MAX_BYTES:
            raise ValueError('The saved pin import is too large.')
        value = json.loads(text)
        if not isinstance(value, dict) or value.get('version') != 1 or not isinstance(value.get('choices'), list):
            raise ValueError('The saved pin import is unsupported.')
        value['snapshot'] = validate_snapshot(value['snapshot'])
        choices = value['choices']
        if len(choices) != len(value['snapshot']['pins']) or any(key is not None and (not isinstance(key, str) or not 0 < len(key) <= 4096) for key in choices):
            raise ValueError('The saved conversation matches are invalid.')
        if len(set(k for k in choices if k)) != len([k for k in choices if k]):
            raise ValueError('The saved conversation matches contain duplicates.')
        return value

    def load(self):
        text = self.secret.password_lookup_sync(self.schema, self.attributes, None)
        if text:
            return self._decode(text)
        if self.legacy_attributes:
            legacy = self.secret.password_lookup_sync(self.legacy_schema, self.legacy_attributes, None)
            if legacy:
                state = self._decode(legacy)
                self.save(state)
                return state
        return dict(version=1, snapshot=None, choices=[])

    def save(self, state):
        text = json.dumps(state, ensure_ascii=True)
        if len(text.encode()) > MAX_BYTES:
            raise ValueError('The saved pin import is too large.')
        if not self.secret.password_store_sync(self.schema, self.attributes, self.secret.COLLECTION_DEFAULT,
                                               'Messages — imported Apple conversation pins', text, None):
            raise RuntimeError('The desktop keyring did not save the imported pins.')
        if self.legacy_attributes:
            try:
                self.secret.password_clear_sync(self.legacy_schema, self.legacy_attributes, None)
            except Exception:
                # The new shared item is already saved; a leftover legacy item
                # is harmless and can be cleaned up on a later migration.
                pass
        return deepcopy(state)
