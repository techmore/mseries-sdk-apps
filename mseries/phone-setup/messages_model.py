"""Presentation state for native Messages; transport stays in BlueFerry."""
from copy import deepcopy
from dataclasses import dataclass
from blueferry.recipients import validate_recipient


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
