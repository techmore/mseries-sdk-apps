"""Presentation state for native Messages; transport stays in BlueFerry."""
from copy import deepcopy
from dataclasses import dataclass
from datetime import datetime, timedelta
import gettext
import re
from zlib import crc32
from blueferry.events import canonical_address
from blueferry.recipients import validate_recipient
from blueferry.text_safety import terminal_text
from blueferry.time_display import format_message_timestamp

_ = gettext.gettext

def contact_index(records):
    result = {}
    for name, phones, emails in records:
        if not name.strip():
            continue
        for address in [*phones, *emails]:
            if key := canonical_address(address):
                result.setdefault(key, set()).add(name.strip())
    return result


def identity_keys(address):
    """Typed identities for one address: its canonical form, plus the North American twin.

    BlueFerry's canonical form already makes "+1 908 642 0729" and "19086420729" the
    same identity. A 10-digit number and its 11-digit "1" form are also one phone in
    the United States and Canada, so both are returned. Other lengths get no twin.
    Names and unparseable text return ().
    """
    key = canonical_address(address)
    if not key:
        return ()
    keys = [key]
    if key.startswith('phone:'):
        digits = key[6:]
        if len(digits) == 10:
            keys.append('phone:1' + digits)
        elif len(digits) == 11 and digits.startswith('1'):
            keys.append('phone:' + digits[1:])
    return tuple(keys)


def lookup_contact_name(address, index):
    """The one phonebook name for this address, or '' when none or several match."""
    for key in identity_keys(address):
        if names := index.get(key, set()):
            return next(iter(names)) if len(names) == 1 else ''
    return ''


def resolve_contact_names(threads, index):
    """Presentation only: never alter thread identities, groups or recipients."""
    for thread in threads:
        if thread.get('is_group') or thread.get('sender_alphanumeric') or len(thread.get('recipients', [])) != 1:
            continue
        if name := lookup_contact_name(thread['recipients'][0], index):
            thread['name'] = name
    return threads


def address_key(address):
    try:
        return validate_recipient(address).casefold()
    except ValueError:
        return ''


def merge_inbox(threads, inbox):
    """Merge a bounded phone snapshot in memory, preserving backend group routes.

    A snapshot message joins the one direct conversation that owns its sender by
    identity (so "+19086420729" and "19086420729" are the same person). If no
    direct conversation owns it, or more than one claims it, it goes to a single
    phone-only conversation for that sender instead of guessing.
    """
    result = {t['key']: deepcopy(t) for t in threads}
    handles = {m.get('handle') for t in result.values() for m in t['messages'] if m.get('handle')}
    owners = {}
    for thread in result.values():
        if not thread.get('is_group') and len(thread.get('recipients', [])) == 1:
            for key in identity_keys(thread['recipients'][0]):
                owners.setdefault(key, []).append(thread)
    phone_only = {}
    for raw in inbox:
        sender = raw.get('sender_phone_norm') or raw.get('sender', '')
        address = address_key(sender)
        if not address:
            continue
        keys = identity_keys(sender)
        candidates = []
        for key in keys:
            for candidate in owners.get(key, []):
                if candidate not in candidates:
                    candidates.append(candidate)
        thread = candidates[0] if len(candidates) == 1 else None
        if thread is None:
            thread = next((phone_only[key] for key in keys if key in phone_only), None)
        if thread is None and raw.get('handle') in handles:
            continue
        if thread is None:
            key = 'inbox:' + address
            thread = dict(key=key, name=raw.get('contact_name') or raw.get('sender') or address,
                          recipients=[address], is_group=False, reply_ready=not bool(raw.get('sender_alphanumeric')),
                          messages=[], last_ts='', inbox_only=True)
            result[key] = thread
            for identity in keys or (address,):
                phone_only[identity] = thread
        # Newer BlueFerry marks sender IDs represented as keypad digits. Never
        # promote those digits into a reply route, even for duplicate snapshots.
        if raw.get('sender_alphanumeric'):
            thread['reply_ready'] = False
            thread['sender_alphanumeric'] = str(raw['sender_alphanumeric'])
            thread['name'] = thread['sender_alphanumeric']
        if raw.get('handle') in handles:
            continue
        thread['messages'].append(dict(handle=raw.get('handle', ''), body=raw.get('body', ''),
                                       timestamp=raw.get('timestamp', ''), outgoing=False,
                                       read=raw.get('read', False), sender=raw.get('sender', ''), phone_list=True))
        if raw.get('read') is False:
            thread['unread_count'] = thread.get('unread_count', 0) + 1
        if raw.get('handle'):
            handles.add(raw['handle'])
    for thread in result.values():
        thread['messages'].sort(key=lambda m: m.get('timestamp', ''))
        if thread['messages']:
            thread['last_ts'] = thread['messages'][-1].get('timestamp', '')
    return sorted(result.values(), key=lambda t: t.get('last_ts', ''), reverse=True)


@dataclass(frozen=True)
class SendAttempt:
    key: str
    body: str
    recipient: str
    token: str
    backend_thread: bool
    # True only after the user confirmed the displayed participant list.
    confirm_group: bool = False


class ConversationState:
    def __init__(self):
        self.selected = ''
        self.drafts = {}
        self.pending = set()

    def prepare(self, thread, confirm_group=False):
        key = thread['key']
        body = self.drafts.get(key, '')
        if key in self.pending or not body.strip():
            raise ValueError('Write a message first.')
        if not thread.get('reply_ready'):
            raise ValueError(reply_block_reason(thread))
        recipients = thread.get('recipients', [])
        backend_thread = not thread.get('inbox_only') and not thread.get('new')
        if not backend_thread and len(recipients) != 1:
            raise ValueError('Choose one recipient.')
        recipient = validate_recipient(recipients[0]) if recipients else ''
        attempt = SendAttempt(key, body, recipient, thread.get('confirmation_token', ''), backend_thread,
                              bool(confirm_group and thread.get('is_group')))
        self.pending.add(key)
        return attempt

    def finish(self, attempt, success):
        self.pending.discard(attempt.key)
        if success and self.drafts.get(attempt.key) == attempt.body:
            self.drafts.pop(attempt.key, None)


# --- Group replies and failures -------------------------------------------
# BlueFerry refuses to message a group until a client has shown its participants
# and the user confirmed them. These helpers let the window do exactly that.

def reply_block_reason(thread):
    if thread.get('is_group'):
        return _('Confirm this group’s participants in Advanced BlueFerry before replying.')
    return _('This sender cannot receive replies.')


def needs_group_confirmation(thread):
    """A replyable group whose participant list the user has not confirmed yet."""
    return bool(thread.get('is_group') and thread.get('reply_ready')
                and thread.get('confirmation_token') and not thread.get('group_confirmed'))


def blocks_group_confirmation(thread):
    """The roster itself is incomplete, so it cannot be confirmed from here."""
    return bool(thread.get('is_group') and thread.get('participants_required'))


def group_roster(thread, index):
    """[(name or '', address)] for every recipient the message would reach."""
    return [(lookup_contact_name(address, index), address) for address in thread.get('recipients', [])]


def send_failure_reason(error):
    """A short, plain reason for a failed send, from BlueFerry's own wording."""
    text = ' '.join(terminal_text(str(error or '')).split())
    lowered = text.lower()
    known = (
        (('no longer exists',), _('this conversation is no longer in BlueFerry’s history')),
        (('unambiguous reply',), _('BlueFerry can’t tell who to reply to')),
        (('participant', 'confirm'), _('the group’s participants need confirming')),
        (('group changed',), _('the group changed; review the participants')),
        (('too many', 'rate limit'), _('too many messages in a short time; wait a minute')),
        (('forbidden', 'denied', 'authoriz'), _('your iPhone isn’t allowing message access; check Bluetooth settings on the phone')),
        (('not connected', 'not ready', 'obex', 'bluetooth', 'map '), _('your iPhone isn’t connected for messages')),
    )
    for needles, reason in known:
        if any(needle in lowered for needle in needles):
            return reason
    return text[:140] if text else _('BlueFerry could not send it')


# --- Presentation helpers -------------------------------------------------
# Everything below is pure: no GTK, no Bluetooth, no clock unless passed in.

# White initials stay at 4.5:1 or better on every tone (asserted in the tests).
AVATAR_TONES = ('#c6262e', '#c2410c', '#9a5b00', '#2f7d04', '#0b7a5b', '#0e7490',
                '#0d52bf', '#4f46e5', '#7239b3', '#b02a73', '#715344', '#485a6c')
# Bubbles from the same person within five minutes read as one run; a centred
# time heading appears after a longer silence or when the day changes.
RUN_GAP = timedelta(minutes=5)
STAMP_GAP = timedelta(minutes=30)


def avatar_tone(identity):
    """Stable colour for a person or group; crc32 is not salted like hash()."""
    return crc32(str(identity).casefold().encode()) % len(AVATAR_TONES)


def initials(name):
    """Up to two initials, or '' when the name has no letters (a bare number)."""
    words = [w for w in str(name).split() if any(c.isalpha() for c in w)]
    letters = [next(c for c in w if c.isalpha()).upper() for w in words]
    if not letters:
        return ''
    return letters[0] if len(letters) == 1 else letters[0] + letters[-1]


def parse_timestamp(value):
    """Naive local datetime for an ISO timestamp, or None when unparseable."""
    if not value:
        return None
    try:
        parsed = datetime.fromisoformat(str(value).strip().replace('Z', '+00:00'))
    except ValueError:
        return None
    return parsed.astimezone().replace(tzinfo=None) if parsed.tzinfo else parsed


def short_stamp(value, now=None):
    """Compact sidebar time: clock today, then Yesterday, weekday, date."""
    parsed = parse_timestamp(value)
    if parsed is None:
        return ''
    now = now or datetime.now()
    days = (now.date() - parsed.date()).days
    if days <= 0:
        return f"{parsed.hour % 12 or 12}:{parsed.minute:02d} {'AM' if parsed.hour < 12 else 'PM'}"
    if days == 1:
        return _('Yesterday')
    if days < 7:
        return (_('Monday'), _('Tuesday'), _('Wednesday'), _('Thursday'), _('Friday'),
                _('Saturday'), _('Sunday'))[parsed.weekday()]
    month = (_('Jan'), _('Feb'), _('Mar'), _('Apr'), _('May'), _('Jun'),
             _('Jul'), _('Aug'), _('Sep'), _('Oct'), _('Nov'), _('Dec'))[parsed.month - 1]
    return f'{month} {parsed.day}' if parsed.year == now.year else f'{month} {parsed.day}, {parsed.year}'


# iOS marks an inline attachment with U+FFFC, and the phone's Bluetooth message
# service sends only text, so a photo or file arrives as that mark or as nothing.
ATTACHMENT_MARK = '\ufffc'
# The phone's inbox listing cuts each message at 128 characters and never says so.
# A listing-only message this long may have lost its tail; live messages are whole.
SHORTENED_AT = 120


def message_parts(message):
    """(text, has_attachment, shortened) for what a bubble should show."""
    body = str(message.get('body') or '')
    attachment = ATTACHMENT_MARK in body or not body.strip()
    text = body.replace(ATTACHMENT_MARK, '').strip() if attachment else body
    shortened = bool(message.get('body_truncated')) or (bool(message.get('phone_list')) and len(text) >= SHORTENED_AT)
    return text, attachment, shortened


def conversation_preview(thread):
    """(one-line preview, sent-by-me) for the newest message in a thread."""
    messages = thread.get('messages') or []
    if not messages:
        return '', False
    last = messages[-1]
    text, attachment, _shortened = message_parts(last)
    preview = ' '.join(text.split())
    if attachment:
        preview = (preview + ' · ' if preview else '') + _('Attachment')
    return preview, bool(last.get('outgoing'))


# --- Tapbacks and quoted replies -------------------------------------------
# The phone's Bluetooth link carries iMessage only as plain text, so a Tapback
# arrives as an ordinary message such as: Liked “Do you have a starlink mini?”
# These helpers fold such a message onto the message it names, and build the
# quote a reply carries. Nothing is guessed: a Tapback whose target cannot be
# found stays a normal bubble.

REACTION_EMOJI = {'Loved': '❤️', 'Liked': '👍', 'Disliked': '👎', 'Laughed at': '😂',
                  'Emphasized': '‼️', 'Questioned': '❓'}
_REMOVED_WORDS = {'heart': 'Loved', 'like': 'Liked', 'dislike': 'Disliked', 'laugh': 'Laughed at',
                  'exclamation mark': 'Emphasized', 'question mark': 'Questioned'}
_QUOTED = r'[“"](.+)[”"]'
_WHO = r'(?:(?P<who>[^“"\n]{1,40}?) )?'
_TAPBACK = re.compile(r'^' + _WHO + r'(?P<verb>Loved|Liked|Disliked|Laughed at|Emphasized|Questioned) ' + _QUOTED + r'$', re.S | re.I)
_EMOJI_TAPBACK = re.compile(r'^' + _WHO + r'Reacted (?P<emoji>\S+) to ' + _QUOTED + r'$', re.S | re.I)
_REMOVED = re.compile(r'^' + _WHO + r'Removed a (?P<word>heart|like|dislike|laugh|exclamation mark|question mark) from ' + _QUOTED + r'$', re.S | re.I)
QUOTE_LIMIT = 80


def _squash(text):
    return ' '.join(str(text or '').split())


def parse_tapback(body):
    """(reaction, quoted text, removed, who) for a Tapback message, else None.

    `who` is the name the phone wrote before the verb ("Derek laughed at …"), or ''.
    """
    text = str(body or '').strip()
    found = _TAPBACK.match(text)
    if found:
        verb = found.group('verb').capitalize() if found.group('verb').lower() != 'laughed at' else 'Laughed at'
        return verb, found.group(found.lastindex), False, (found.group('who') or '').strip()
    found = _EMOJI_TAPBACK.match(text)
    if found:
        return found.group('emoji'), found.group(found.lastindex), False, (found.group('who') or '').strip()
    found = _REMOVED.match(text)
    if found:
        return _REMOVED_WORDS[found.group('word').lower()], found.group(found.lastindex), True, (found.group('who') or '').strip()
    return None


def reaction_symbol(reaction):
    return REACTION_EMOJI.get(reaction, reaction)


def _names_message(body, quoted):
    """True when `quoted` is `body`, or the phone's shortened copy of it."""
    body, quoted = _squash(body), _squash(quoted)
    if not quoted:
        return False
    if quoted.endswith('…') or quoted.endswith('...'):
        return body.startswith(quoted.rstrip('….').rstrip())
    return body == quoted


def fold_reactions(messages):
    """Copies of `messages` with each Tapback folded onto the message it names.

    A folded message gains `reactions`: [{'reaction', 'by'}], one per person
    (a newer Tapback from the same person replaces the older one, and a removal
    deletes it). `by` is the sender's name, 'You' for ours, or '' when a direct
    chat does not say.
    """
    folded = []
    for message in messages:
        copy = dict(message)
        parsed = parse_tapback(copy.get('body'))
        target = None
        if parsed:
            reaction, quoted, removed, named = parsed
            target = next((earlier for earlier in reversed(folded)
                           if _names_message(earlier.get('body'), quoted)), None)
        if target is None:
            folded.append(copy)
            continue
        who = 'You' if copy.get('outgoing') else (named or str(copy.get('sender') or ''))
        kept = [r for r in target.get('reactions', []) if r['by'] != who]
        if not removed:
            kept.append({'reaction': reaction, 'by': who})
        target['reactions'] = kept
    return folded


def reaction_summary(reactions):
    """(symbols, tooltip) for a reaction chip, in the order they were given."""
    symbols = ''.join(dict.fromkeys(reaction_symbol(r['reaction']) for r in reactions))
    by_kind = []
    for entry in reactions:
        who = entry['by'] or _('Them')
        by_kind.append('%s %s' % (reaction_symbol(entry['reaction']), who))
    return symbols, ' · '.join(by_kind)


def quote_snippet(message, is_group=False):
    """A one-line quote of a message for a reply, naming the sender in groups."""
    text, attachment, _shortened = message_parts(message)
    line = _squash(text) or (_('Attachment') if attachment else '')
    if len(line) > QUOTE_LIMIT:
        line = line[:QUOTE_LIMIT - 1].rstrip() + '…'
    if is_group and not message.get('outgoing') and message.get('sender'):
        return '%s: “%s”' % (message['sender'], line)
    return '“%s”' % line


def reply_body(quote, body):
    """What is sent for a reply: a quote line, then the user's words."""
    return '↪ %s\n%s' % (quote, body) if quote else body


def layout_messages(messages, is_group=False, now=None):
    """Turn a message list into render items: centred time headings and bubbles.

    Each bubble item carries its place in a run (single/first/middle/last) so
    the view can tighten spacing and put the speech-bubble tail only on the
    last bubble. Messages whose timestamp cannot be parsed are never grouped;
    they keep their own caption as before.
    """
    messages = fold_reactions(messages)
    items = []
    run = []
    previous = None  # (datetime, direction identity) of the prior message

    def close_run():
        for index, entry in enumerate(run):
            if len(run) == 1:
                entry['position'] = 'single'
            else:
                entry['position'] = 'first' if index == 0 else 'last' if index == len(run) - 1 else 'middle'
        run.clear()

    for message in messages:
        moment = parse_timestamp(message.get('timestamp'))
        outgoing = bool(message.get('outgoing'))
        who = (outgoing, '' if outgoing or not is_group else message.get('sender', ''))
        item = dict(kind='message', message=message, position='single', caption='',
                    show_sender=False, moment=moment)
        if moment is None:
            close_run()
            item['caption'] = message.get('display_timestamp') or message.get('timestamp', '')
            item['show_sender'] = is_group and not outgoing and bool(message.get('sender'))
            items.append(item)
            previous = None
            continue
        gap = moment - previous[0] if previous else None
        new_heading = previous is None or moment.date() != previous[0].date() or gap >= STAMP_GAP
        if new_heading:
            close_run()
            items.append(dict(kind='stamp', text=format_message_timestamp(message.get('timestamp'), now=now)))
        elif who != previous[1] or gap >= RUN_GAP:
            close_run()
        if not run:
            item['show_sender'] = is_group and not outgoing and bool(message.get('sender'))
        run.append(item)
        items.append(item)
        previous = (moment, who)
    close_run()
    return items


@dataclass
class OutboxEntry:
    key: str
    body: str
    baseline: int
    state: str = 'sending'


class Outbox:
    """Messages this window sent that the backend history may not show yet.

    An entry disappears once the backend history contains one more outgoing
    message with the same text than it did when the entry was created, so a
    bubble is never shown twice. Entries live in memory only, like drafts.
    """

    def __init__(self):
        self.entries = []

    @staticmethod
    def _norm(body):
        return str(body).replace('\r\n', '\n').strip()

    @classmethod
    def _echoed(cls, messages, body):
        wanted = cls._norm(body)
        return sum(1 for m in messages if m.get('outgoing') and cls._norm(m.get('body', '')) == wanted)

    def add(self, key, body, messages):
        wanted = self._norm(body)
        earlier = sum(1 for e in self.entries if e.key == key and self._norm(e.body) == wanted)
        entry = OutboxEntry(key, body, self._echoed(messages, body) + earlier)
        self.entries.append(entry)
        return entry

    def delivered(self, entry):
        entry.state = 'sent'

    def failed(self, entry):
        if entry in self.entries:
            self.entries.remove(entry)

    def rekey(self, old, new):
        for entry in self.entries:
            if entry.key == old:
                entry.key = new

    def visible(self, key, messages):
        """Entries for this thread the backend has not echoed back yet."""
        shown = []
        for entry in list(self.entries):
            if entry.key != key:
                continue
            if self._echoed(messages, entry.body) > entry.baseline:
                self.entries.remove(entry)
            else:
                shown.append(entry)
        return shown
