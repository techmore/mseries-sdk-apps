import unittest
from datetime import datetime
from messages_model import (AVATAR_TONES, ConversationState, Outbox, avatar_tone, conversation_preview,
                            initials, layout_messages, merge_inbox, parse_timestamp, short_stamp,
                            contact_index, resolve_contact_names, blocks_group_confirmation, group_roster,
                            lookup_contact_name, needs_group_confirmation, send_failure_reason)


def thread(key='alice', **values):
    return dict(key=key, recipients=['+15550100123'], reply_ready=True, messages=[], **values)


class RoutingTests(unittest.TestCase):
    def test_completion_cannot_clear_other_conversation_draft(self):
        model = ConversationState()
        model.drafts = dict(alice='First message', bob='Other conversation')
        attempt = model.prepare(thread())
        model.selected = 'bob'
        model.finish(attempt, True)
        self.assertEqual(model.drafts, dict(bob='Other conversation'))

    def test_failure_keeps_draft_and_allows_explicit_retry(self):
        model = ConversationState()
        model.drafts['alice'] = 'Keep this draft'
        attempt = model.prepare(thread())
        with self.assertRaises(ValueError):
            model.prepare(thread())
        model.finish(attempt, False)
        self.assertEqual(model.prepare(thread()).body, 'Keep this draft')

    def test_text_typed_during_send_survives_completion(self):
        model = ConversationState()
        model.drafts['alice'] = 'First'
        attempt = model.prepare(thread())
        model.drafts['alice'] = 'Next thought'
        model.finish(attempt, True)
        self.assertEqual(model.drafts['alice'], 'Next thought')

    def test_group_route_uses_backend_identity_and_roster_token(self):
        model = ConversationState()
        model.drafts['family'] = 'Hi'
        attempt = model.prepare(thread('family', is_group=True, confirmation_token='roster-v1'))
        self.assertTrue(attempt.backend_thread)
        self.assertEqual(attempt.token, 'roster-v1')

    def test_unconfirmed_group_and_injected_address_rejected(self):
        model = ConversationState()
        model.drafts['alice'] = 'Hi'
        t = thread();t['reply_ready'] = False
        with self.assertRaises(ValueError): model.prepare(t)
        t = thread(new=True);t['recipients'] = ['+15550100123\r\nTEL:other']
        with self.assertRaises(ValueError): model.prepare(t)


class SnapshotTests(unittest.TestCase):
    def test_phonebook_names_use_unique_addresses_without_changing_routes(self):
        index=contact_index([('Alice',['+1 (555) 010-0123'],[]),('Email Person',[],['PERSON@example.com'])])
        rows=[thread(name='15550100123'),dict(key='email',name='person@example.com',recipients=['person@example.com'],messages=[])]
        resolve_contact_names(rows,index)
        self.assertEqual(rows[0]['name'],'Alice')
        self.assertEqual(rows[0]['recipients'],['+15550100123'])
        self.assertEqual(rows[1]['name'],'Email Person')

    def test_ambiguous_contacts_groups_and_international_suffixes_are_not_guessed(self):
        index=contact_index([('Alice',['15550100123'],[]),('Other',['15550100123'],[]),('International',['4415550100123'],[])])
        rows=[thread(name='Number'),thread('group',name='Family',is_group=True)]
        resolve_contact_names(rows,index)
        self.assertEqual([r['name'] for r in rows],['Number','Family'])
        self.assertEqual(resolve_contact_names([thread(name='Number')],contact_index([('International',['4415550100123'],[])]))[0]['name'],'Number')

    def test_cached_nanp_contact_and_explicit_unread_snapshot(self):
        rows=merge_inbox([], [dict(handle='fresh',sender='+15550100123',body='Fixture',read=False)])
        resolve_contact_names(rows,contact_index([('Alice',['5550100123'],[])]))
        self.assertEqual(rows[0]['name'],'Alice')
        self.assertEqual(rows[0]['unread_count'],1)
        read=merge_inbox([], [dict(handle='read',sender='+15550100123',body='Fixture',read=True)])
        self.assertEqual(read[0].get('unread_count',0),0)

    def test_known_group_message_does_not_become_direct_thread(self):
        t = thread('group', is_group=True)
        t['messages'] = [dict(handle='123', body='Group', timestamp='2026-10-20')]
        result = merge_inbox([t], [dict(handle='123', sender='+15550100124', body='Group')])
        self.assertEqual(len(result), 1)
        self.assertEqual(len(t['messages']), 1)

    def test_missed_snapshot_merges_with_live_without_duplicate(self):
        t = thread();t['messages'] = [dict(handle='new', body='Live', timestamp='2026-10-20')]
        inbox = [dict(handle='old', sender='+15550100123', body='Missed', timestamp='2026-10-19'), dict(handle='new', sender='+15550100123', body='Live')]
        result = merge_inbox([t], inbox)
        self.assertEqual([m['body'] for m in result[0]['messages']], ['Missed', 'Live'])
        self.assertEqual(len(t['messages']), 1)

    def test_unknown_sender_does_not_invent_a_route(self):
        self.assertEqual(merge_inbox([], [dict(handle='bad', sender='Bank Alert', body='Test')]), [])


NOW = datetime(2026, 10, 20, 15, 0)


def at(hour, minute=0, day=20):
    return '2026-10-%02dT%02d:%02d:00' % (day, hour, minute)


def msg(body, when, outgoing=False, sender=''):
    return dict(body=body, timestamp=when, outgoing=outgoing, sender=sender)


class AvatarTests(unittest.TestCase):
    def test_initials(self):
        self.assertEqual(initials('Alex Morgan'), 'AM')
        self.assertEqual(initials('Cher'), 'C')
        self.assertEqual(initials('Mary Jane van Dyke'), 'MD')
        self.assertEqual(initials('élodie'), 'É')
        self.assertEqual(initials('+1 555 0100'), '', 'a bare number has no initials')
        self.assertEqual(initials(''), '')

    def test_tone_is_stable_and_in_range(self):
        self.assertEqual(avatar_tone('+15550100123'), avatar_tone('+15550100123'))
        self.assertEqual(avatar_tone('Alex'), avatar_tone('ALEX'))
        for who in ('a', 'b', '+15550100124', 'Family'):
            self.assertIn(avatar_tone(who), range(len(AVATAR_TONES)))
        # Different people should not all collapse onto one colour.
        self.assertGreater(len({avatar_tone('person%d' % i) for i in range(50)}), 6)

    def test_every_tone_keeps_white_initials_readable(self):
        def luminance(color):
            channels = [int(color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
            r, g, b = [c / 12.92 if c <= .03928 else ((c + .055) / 1.055) ** 2.4 for c in channels]
            return .2126 * r + .7152 * g + .0722 * b
        for color in AVATAR_TONES:
            self.assertGreaterEqual(1.05 / (luminance(color) + .05), 4.5, color)


class TimeTests(unittest.TestCase):
    def test_parse_handles_naive_aware_and_garbage(self):
        self.assertEqual(parse_timestamp('2026-10-20T14:42:00'), datetime(2026, 10, 20, 14, 42))
        self.assertIsNotNone(parse_timestamp('2026-10-20T14:42:00Z'))
        self.assertIsNone(parse_timestamp('Today, 11:42'))
        self.assertIsNone(parse_timestamp(''))
        self.assertIsNone(parse_timestamp(None))

    def test_short_stamp_ladder(self):
        self.assertEqual(short_stamp(at(14, 42), NOW), '2:42 PM')
        self.assertEqual(short_stamp(at(0, 5), NOW), '12:05 AM')
        self.assertEqual(short_stamp(at(12, 0), NOW), '12:00 PM')
        self.assertEqual(short_stamp(at(9, day=19), NOW), 'Yesterday')
        self.assertEqual(short_stamp(at(9, day=16), NOW), 'Friday')
        self.assertEqual(short_stamp(at(9, day=3), NOW), 'Oct 3')
        self.assertEqual(short_stamp('2025-12-25T09:00:00', NOW), 'Dec 25, 2025')
        self.assertEqual(short_stamp('nonsense', NOW), '')

    def test_preview_prefers_latest_and_flags_own_message(self):
        self.assertEqual(conversation_preview(dict(messages=[])), ('', False))
        self.assertEqual(conversation_preview(dict(messages=[msg('a', at(9)), msg('  two\n lines  ', at(10), True)])), ('two lines', True))


class LayoutTests(unittest.TestCase):
    def shape(self, messages, **kwargs):
        return [(i['text'] if i['kind'] == 'stamp' else (i['message']['body'], i['position'])) for i in layout_messages(messages, now=NOW, **kwargs)]

    def test_run_positions_and_heading(self):
        result = self.shape([msg('a', at(14, 0)), msg('b', at(14, 1)), msg('c', at(14, 2)), msg('d', at(14, 3), True)])
        self.assertEqual(result, ['Today at 2:00 PM', ('a', 'first'), ('b', 'middle'), ('c', 'last'), ('d', 'single')])

    def test_long_silence_adds_heading_and_splits_run(self):
        result = self.shape([msg('a', at(9, 0)), msg('b', at(9, 10)), msg('c', at(14, 0))])
        self.assertEqual(result, ['Today at 9:00 AM', ('a', 'single'), ('b', 'single'), 'Today at 2:00 PM', ('c', 'single')])

    def test_new_day_adds_heading_even_for_a_short_gap(self):
        result = self.shape([msg('a', '2026-10-19T23:58:00'), msg('b', '2026-10-20T00:01:00')])
        self.assertEqual([r for r in result if isinstance(r, str)], ['Yesterday at 11:58 PM', 'Today at 12:01 AM'])

    def test_unparseable_timestamps_are_never_grouped_and_keep_caption(self):
        items = layout_messages([msg('a', 'Now'), msg('b', 'Now')], now=NOW)
        self.assertEqual([i['kind'] for i in items], ['message', 'message'])
        self.assertEqual([i['position'] for i in items], ['single', 'single'])
        self.assertEqual(items[0]['caption'], 'Now')

    def test_group_names_the_sender_once_per_run(self):
        items = layout_messages([msg('a', at(14, 0), sender='+1'), msg('b', at(14, 1), sender='+1'),
                                 msg('c', at(14, 2), sender='+2'), msg('d', at(14, 3), True)], is_group=True, now=NOW)
        shown = [(i['message']['body'], i['show_sender']) for i in items if i['kind'] == 'message']
        self.assertEqual(shown, [('a', True), ('b', False), ('c', True), ('d', False)])

    def test_direct_chat_never_shows_a_sender_name(self):
        items = layout_messages([msg('a', at(14, 0), sender='+1')], now=NOW)
        self.assertFalse(any(i.get('show_sender') for i in items))

    def test_empty_conversation(self):
        self.assertEqual(layout_messages([], now=NOW), [])


class OutboxTests(unittest.TestCase):
    def history(self, *bodies):
        return [msg(b, at(14), True) for b in bodies]

    def test_pending_bubble_disappears_exactly_when_history_echoes_it(self):
        box = Outbox()
        entry = box.add('k', 'On my way', self.history())
        self.assertEqual(box.visible('k', self.history()), [entry])
        box.delivered(entry)
        self.assertEqual(box.visible('k', self.history()), [entry], 'sent but not yet echoed stays visible')
        self.assertEqual(box.visible('k', self.history('On my way')), [])
        self.assertEqual(box.entries, [])

    def test_never_doubles_a_message_already_in_history(self):
        box = Outbox()
        box.add('k', 'ok', self.history('ok'))
        self.assertEqual(len(box.visible('k', self.history('ok'))), 1, 'older identical message is not the echo')
        self.assertEqual(box.visible('k', self.history('ok', 'ok')), [])

    def test_two_identical_sends_reconcile_one_at_a_time(self):
        box = Outbox()
        first = box.add('k', 'ok', self.history())
        second = box.add('k', 'ok', self.history())
        self.assertEqual(box.visible('k', self.history()), [first, second])
        self.assertEqual(box.visible('k', self.history('ok')), [second])
        self.assertEqual(box.visible('k', self.history('ok', 'ok')), [])

    def test_whitespace_and_line_endings_do_not_defeat_matching(self):
        box = Outbox()
        box.add('k', 'hi\r\nthere  ', self.history())
        self.assertEqual(box.visible('k', self.history('hi\nthere')), [])

    def test_failure_removes_the_entry_and_other_threads_are_untouched(self):
        box = Outbox()
        bad = box.add('k', 'x', [])
        keep = box.add('other', 'x', [])
        box.failed(bad)
        box.failed(bad)
        self.assertEqual(box.visible('k', []), [])
        self.assertEqual(box.visible('other', []), [keep])

    def test_rekey_follows_a_new_conversation_to_its_backend_identity(self):
        box = Outbox()
        entry = box.add('new:+1555', 'hello', [])
        box.rekey('new:+1555', 'thread-7')
        self.assertEqual(box.visible('thread-7', []), [entry])
        self.assertEqual(box.visible('new:+1555', []), [])


def group(**values):
    base = dict(key='g', name='Family', is_group=True, reply_ready=True, confirmation_token='roster-v1',
                group_confirmed=False, recipients=['+15550100126', '+15550100127'], messages=[])
    base.update(values)
    return base


class GroupReplyTests(unittest.TestCase):
    def test_only_a_replyable_unconfirmed_group_needs_confirmation(self):
        self.assertTrue(needs_group_confirmation(group()))
        self.assertFalse(needs_group_confirmation(group(group_confirmed=True)))
        self.assertFalse(needs_group_confirmation(group(reply_ready=False)))
        self.assertFalse(needs_group_confirmation(group(confirmation_token='')))
        self.assertFalse(needs_group_confirmation(group(is_group=False)))

    def test_incomplete_roster_cannot_be_confirmed_here(self):
        self.assertTrue(blocks_group_confirmation(group(participants_required=True)))
        self.assertFalse(blocks_group_confirmation(group()))
        self.assertFalse(blocks_group_confirmation(group(is_group=False, participants_required=True)))

    def test_roster_shows_names_only_when_the_phonebook_is_unambiguous(self):
        index = contact_index([('Alex', ['+15550100126'], []), ('Sam', ['+15550100127'], []), ('Other Sam', ['+15550100127'], [])])
        self.assertEqual(group_roster(group(), index), [('Alex', '+15550100126'), ('', '+15550100127')])
        self.assertEqual(lookup_contact_name('+15550100126', index), 'Alex')
        self.assertEqual(lookup_contact_name('not an address', index), '')

    def test_confirmation_is_carried_only_for_groups(self):
        state = ConversationState()
        state.drafts['g'] = 'Hi'
        self.assertFalse(state.prepare(group()).confirm_group, 'default never confirms')
        state.pending.clear()
        self.assertTrue(state.prepare(group(), confirm_group=True).confirm_group)
        state.pending.clear()
        state.drafts['alice'] = 'Hi'
        self.assertFalse(state.prepare(thread(), confirm_group=True).confirm_group, 'a direct chat has nothing to confirm')


class FailureReasonTests(unittest.TestCase):
    def test_daemon_wording_becomes_plain_reasons(self):
        cases = {
            'thread no longer exists in local history': 'no longer in BlueFerry',
            'thread has no unambiguous reply destination': 'can’t tell who to reply to',
            'confirm the displayed participant list before replying to this group': 'participants need confirming',
            'the group changed; refresh the conversation': 'group changed',
            'OBEX connect failed: Forbidden': 'isn’t allowing message access',
            'MAP session not connected': 'isn’t connected',
            'too many requests, rate limit exceeded': 'too many messages',
        }
        for raw, expected in cases.items():
            self.assertIn(expected, send_failure_reason(raw), raw)

    def test_unknown_errors_are_shown_short_single_line_and_never_empty(self):
        self.assertEqual(send_failure_reason('Something\nodd   happened'), 'Something odd happened')
        self.assertEqual(len(send_failure_reason('x' * 500)), 140)
        self.assertEqual(send_failure_reason(''), 'BlueFerry could not send it')
        self.assertEqual(send_failure_reason(None), 'BlueFerry could not send it')
        self.assertNotIn('\x1b', send_failure_reason('red \x1b[31mtext'))


if __name__ == '__main__':
    unittest.main()
