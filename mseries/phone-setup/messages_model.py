"""Presentation state for native Messages; transport stays in BlueFerry."""
from copy import deepcopy
from dataclasses import dataclass
from datetime import datetime, timedelta
import gettext
from zlib import crc32
from blueferry.recipients import validate_recipient
from blueferry.time_display import format_message_timestamp

_ = gettext.gettext


def address_key(address):
    try:
        return validate_recipient(address).casefold()
    except ValueError:
        return ''


def merge_inbox(threads, inbox):
    """Merge a bounded phone snapshot in memory, preserving backend group routes."""
    result = {t['key']: deepcopy(t) for t in threads}
    handles = {m.get('handle') for t in result.values() for m in t['messages'] if m.get('handle')}
    by_address = {}
    for thread in result.values():
        if not thread.get('is_group') and len(thread.get('recipients', [])) == 1:
            by_address[address_key(thread['recipients'][0])] = thread
    for raw in inbox:
        if raw.get('handle') in handles:
            continue
        address = address_key(raw.get('sender_phone_norm') or raw.get('sender', ''))
        if not address:
            continue
        thread = by_address.get(address)
        if thread is None:
            key = 'inbox:' + address
            thread = dict(key=key, name=raw.get('contact_name') or raw.get('sender') or address,
                          recipients=[address], is_group=False, reply_ready=True,
                          messages=[], last_ts='', inbox_only=True)
            result[key] = thread
            by_address[address] = thread
        thread['messages'].append(dict(handle=raw.get('handle', ''), body=raw.get('body', ''),
                                       timestamp=raw.get('timestamp', ''), outgoing=False,
                                       read=raw.get('read', False), sender=raw.get('sender', '')))
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


class ConversationState:
    def __init__(self):
        self.selected = ''
        self.drafts = {}
        self.pending = set()

    def prepare(self, thread):
        key = thread['key']
        body = self.drafts.get(key, '')
        if key in self.pending or not body.strip():
            raise ValueError('Write a message first.')
        if not thread.get('reply_ready'):
            raise ValueError('Confirm this conversation’s participants in BlueFerry before replying.')
        recipients = thread.get('recipients', [])
        backend_thread = not thread.get('inbox_only') and not thread.get('new')
        if not backend_thread and len(recipients) != 1:
            raise ValueError('Choose one recipient.')
        recipient = validate_recipient(recipients[0]) if recipients else ''
        attempt = SendAttempt(key, body, recipient, thread.get('confirmation_token', ''), backend_thread)
        self.pending.add(key)
        return attempt

    def finish(self, attempt, success):
        self.pending.discard(attempt.key)
        if success and self.drafts.get(attempt.key) == attempt.body:
            self.drafts.pop(attempt.key, None)


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


def conversation_preview(thread):
    """(one-line preview, sent-by-me) for the newest message in a thread."""
    messages = thread.get('messages') or []
    if not messages:
        return '', False
    last = messages[-1]
    return ' '.join(str(last.get('body', '')).split()), bool(last.get('outgoing'))


def layout_messages(messages, is_group=False, now=None):
    """Turn a message list into render items: centred time headings and bubbles.

    Each bubble item carries its place in a run (single/first/middle/last) so
    the view can tighten spacing and put the speech-bubble tail only on the
    last bubble. Messages whose timestamp cannot be parsed are never grouped;
    they keep their own caption as before.
    """
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
