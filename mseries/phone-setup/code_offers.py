"""Short-lived, explicit-use verification-code offers. No disk or clipboard writes."""
from dataclasses import dataclass, field
from datetime import datetime, timezone
import re
import secrets
from urllib.parse import urlsplit
from blueferry.otp import extract_otp

MAX_AGE = 300
_FIELD = re.compile(r'\b(?:otp|2fa|mfa|passcode)\b|(?:verification|security|authentication|confirmation|one[- ]?time|sms|email).{0,28}(?:code|pin)|(?:code|pin).{0,28}(?:verification|authentication|one[- ]?time)', re.I)
_BOUND = re.compile(r'^@([a-zA-Z0-9.-]+)\s+#([A-Za-z0-9-]+)(?:\s+@([a-zA-Z0-9.-]+))?\s*$', re.M)


def code_field(name, attributes, allow_plain=False):
    values = {str(k).lower(): str(v) for k, v in attributes.items()}
    if 'one-time-code' in values.get('autocomplete', '').split():
        return True
    text = ' '.join([name, *(values.get(k, '') for k in ('placeholder', 'name', 'id', 'description'))])
    return bool(_FIELD.search(text) or (allow_plain and re.search(r'\b(?:code|pin)\b', text, re.I)))


def host_from_url(value):
    try:
        parsed = urlsplit(value)
        return (parsed.hostname or '').lower() if parsed.scheme == 'https' else ''
    except ValueError:
        return ''


@dataclass(frozen=True)
class Offer:
    id: str
    sender: str
    timestamp: float
    code: str = field(repr=False)
    domain: str = ''


class Offers:
    def __init__(self, now=lambda: datetime.now(timezone.utc).timestamp()):
        self.now = now
        self.items = {}
        self.used = set()

    def update(self, messages):
        result = {}
        for raw in messages[:20]:
            try:
                stamp = datetime.fromisoformat(raw.get('timestamp', ''))
                if stamp.tzinfo is None:
                    stamp = stamp.astimezone()
                seconds = stamp.timestamp()
            except (TypeError, ValueError):
                continue
            if not 0 <= self.now() - seconds <= MAX_AGE:
                continue
            code = extract_otp(raw.get('body', ''))
            if not code:
                continue
            key = (str(raw.get('handle', '')), seconds)
            if not key[0] or key in self.used:
                continue
            domain = ''
            binding = _BOUND.search(raw.get('body', ''))
            if binding:
                # Embedded-frame binding cannot safely be proven by a generic
                # desktop accessibility field, so never fill it through this path.
                if binding[3] or binding[2].replace('-', '') != code:
                    continue
                domain = binding[1].lower()
            old = self.items.get(key)
            result[key] = old if old and old.code == code and old.domain == domain else Offer(secrets.token_urlsafe(12), str(raw.get('contact_name') or raw.get('sender') or 'iPhone')[:100], seconds, code, domain)
        self.items = result
        self.used = {key for key in self.used if 0 <= self.now() - key[1] <= MAX_AGE}

    def available(self, origin=None):
        host = host_from_url(origin or '')
        return sorted((o for o in self.items.values() if 0 <= self.now() - o.timestamp <= MAX_AGE and (not o.domain or o.domain == host)), key=lambda o: o.timestamp, reverse=True)[:3]

    def get(self, identity, origin=None):
        return next((o for o in self.available(origin) if o.id == identity), None)

    def consume(self, identity):
        for key, offer in list(self.items.items()):
            if offer.id == identity:
                self.used.add(key)
                del self.items[key]
