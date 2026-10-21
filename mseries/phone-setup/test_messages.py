import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime
import gui_safety
import messages_settings
from messages_model import (AVATAR_TONES, ConversationState, Outbox, avatar_tone, conversation_preview,
                            initials, layout_messages, merge_inbox, message_parts, parse_timestamp, short_stamp,
                            contact_index, resolve_contact_names, blocks_group_confirmation, group_roster,
                            lookup_contact_name, needs_group_confirmation, send_failure_reason, identity_keys)


def thread(key='alice', **values):
    return dict(key=key, recipients=['+15550100123'], reply_ready=True, messages=[], **values)


class MessageContentTests(unittest.TestCase):
    def test_plain_text_is_shown_whole_and_untouched(self):
        body = '  two spaces kept\n' + 'x' * 500
        self.assertEqual(message_parts(dict(body=body)), (body, False, False))

    def test_attachment_only_messages_are_recognised(self):
        for body in ('', '   ', None, '\ufffc', ' \ufffc\ufffc '):
            self.assertEqual(message_parts(dict(body=body)), ('', True, False), repr(body))

    def test_an_attachment_mark_inside_text_keeps_the_text(self):
        self.assertEqual(message_parts(dict(body='Look\ufffc')), ('Look', True, False))

    def test_only_listing_messages_at_the_phone_cap_may_be_shortened(self):
        long = 'y' * 128
        self.assertTrue(message_parts(dict(body=long, phone_list=True))[2])
        self.assertFalse(message_parts(dict(body=long))[2], 'a live message is whole')
        self.assertFalse(message_parts(dict(body='y' * 40, phone_list=True))[2])
        self.assertTrue(message_parts(dict(body='cut', body_truncated=True))[2], 'the backend flag is honoured')

    def test_snapshot_messages_are_marked_as_coming_from_the_phone_listing(self):
        rows = merge_inbox([], [dict(handle='h', sender='+15550100123', body='z' * 128)])
        message = rows[0]['messages'][0]
        self.assertTrue(message['phone_list'] and message_parts(message)[2])

    def test_preview_names_an_attachment_instead_of_showing_nothing(self):
        for body, expected in (('\ufffc', 'Attachment'), ('Photo\ufffc', 'Photo · Attachment'), ('Hi', 'Hi')):
            self.assertEqual(conversation_preview(dict(messages=[dict(body=body)]))[0], expected)


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


class TapbackAndQuoteTests(unittest.TestCase):
    def thread(self, *rows):
        return [dict(body=b, outgoing=o, sender=who, timestamp='2026-10-21T10:%02d:00+00:00' % i) for i, (b, o, who) in enumerate(rows)]

    def test_a_tapback_folds_onto_the_message_it_names(self):
        from messages_model import fold_reactions
        out = fold_reactions(self.thread(('Do you have a starlink mini?', False, ''), ('Liked “Do you have a starlink mini?”', True, '')))
        self.assertEqual(len(out), 1)
        self.assertEqual(out[0]['reactions'], [{'reaction': 'Liked', 'by': 'You'}])

    def test_the_phones_named_and_shortened_forms_fold_too(self):
        from messages_model import fold_reactions
        long = 'Racism incoming I want Rupert to be so good at saving money Jason gives the approval to call it'
        out = fold_reactions(self.thread((long, False, 'Derek'), ('Derek laughed at “%s…”' % long[:60], False, '')))
        self.assertEqual(out[0]['reactions'], [{'reaction': 'Laughed at', 'by': 'Derek'}])

    def test_a_newer_tapback_replaces_and_a_removal_deletes(self):
        from messages_model import fold_reactions
        base = ('Dinner at six', False, 'Jan')
        out = fold_reactions(self.thread(base, ('Liked “Dinner at six”', False, 'Sam'), ('Loved “Dinner at six”', False, 'Sam'), ('Liked “Dinner at six”', False, 'Jan')))
        self.assertEqual([(r['by'], r['reaction']) for r in out[0]['reactions']], [('Sam', 'Loved'), ('Jan', 'Liked')])
        out = fold_reactions(self.thread(base, ('Liked “Dinner at six”', False, 'Sam'), ('Removed a like from “Dinner at six”', False, 'Sam')))
        self.assertEqual(out[0]['reactions'], [])

    def test_an_unmatched_tapback_stays_a_normal_message(self):
        from messages_model import fold_reactions
        rows = self.thread(('Hello', False, ''), ('Liked “Something we never saw”', False, ''))
        out = fold_reactions(rows)
        self.assertEqual([m['body'] for m in out], ['Hello', 'Liked “Something we never saw”'])
        self.assertNotIn('reactions', out[0])

    def test_folding_never_mutates_the_callers_messages(self):
        from messages_model import fold_reactions
        rows = self.thread(('Hi', False, ''), ('Liked “Hi”', True, ''))
        fold_reactions(rows)
        self.assertNotIn('reactions', rows[0])
        self.assertEqual(len(rows), 2)

    def test_a_folded_tapback_adds_no_bubble_to_the_layout(self):
        from messages_model import layout_messages
        items = layout_messages(self.thread(('Hi', False, ''), ('Liked “Hi”', True, '')))
        self.assertEqual([i['kind'] for i in items], ['stamp', 'message'])

    def test_quote_snippets_and_reply_bodies(self):
        from messages_model import quote_snippet, reply_body
        self.assertEqual(quote_snippet(dict(body='Starlink\nmini ?')), '“Starlink mini ?”')
        self.assertEqual(quote_snippet(dict(body='Hi', sender='Jan'), is_group=True), 'Jan: “Hi”')
        self.assertEqual(quote_snippet(dict(body='Hi', sender='Jan', outgoing=True), is_group=True), '“Hi”')
        self.assertTrue(quote_snippet(dict(body='x' * 300)).endswith('…”'))
        self.assertLessEqual(len(quote_snippet(dict(body='x' * 300))), 83)
        self.assertEqual(reply_body('“Hi”', 'Yes'), '↪ “Hi”\nYes')
        self.assertEqual(reply_body('', 'Yes'), 'Yes')


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


class SettingsTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.path = os.path.join(self.dir, 'mseries', 'messages.json')

    def test_missing_corrupt_and_wrongly_typed_files_fall_back_to_defaults(self):
        self.assertEqual(messages_settings.load(self.path), messages_settings.DEFAULTS)
        os.makedirs(os.path.dirname(self.path))
        for content in ('not json', '[]', '{"enter_sends": "yes"}', '{"enter_sends": 1}', ''):
            with open(self.path, 'w') as out:
                out.write(content)
            self.assertEqual(messages_settings.load(self.path), {'enter_sends': True}, content)

    def test_round_trip_keeps_only_known_boolean_switches(self):
        saved = messages_settings.save({'enter_sends': True, 'body': 'secret text', 'recipient': '+15550100123'}, self.path)
        self.assertEqual(saved, {'enter_sends': True})
        self.assertEqual(json.load(open(self.path)), {'enter_sends': True}, 'nothing but the switch is written')
        self.assertEqual(messages_settings.load(self.path), {'enter_sends': True})

    def test_file_and_folder_are_private_and_no_temp_file_is_left(self):
        messages_settings.save({'enter_sends': True}, self.path)
        self.assertEqual(stat.S_IMODE(os.stat(self.path).st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(os.stat(os.path.dirname(self.path)).st_mode), 0o700)
        self.assertEqual(os.listdir(os.path.dirname(self.path)), ['messages.json'])

    def test_unknown_keys_in_an_existing_file_are_never_loaded(self):
        os.makedirs(os.path.dirname(self.path))
        with open(self.path, 'w') as out:
            json.dump({'enter_sends': True, 'evil': True}, out)
        self.assertEqual(messages_settings.load(self.path), {'enter_sends': True})

    def test_default_path_honours_xdg_config_home(self):
        old = os.environ.get('XDG_CONFIG_HOME')
        os.environ['XDG_CONFIG_HOME'] = self.dir
        try:
            self.assertEqual(str(messages_settings.config_path()), os.path.join(self.dir, 'mseries', 'messages.json'))
        finally:
            if old is None:
                del os.environ['XDG_CONFIG_HOME']
            else:
                os.environ['XDG_CONFIG_HOME'] = old


class SnapshotIdentityTests(unittest.TestCase):
    """The phone's inbox snapshot spells a number differently than BlueFerry's threads do."""

    def direct(self, key, recipient, **values):
        return dict(key=key, name='Mike', recipients=[recipient], is_group=False, reply_ready=True, messages=[], last_ts='', **values)

    def snap(self, handle, sender, when, **values):
        return dict(dict(handle=handle, sender=sender, body=handle, timestamp=when, read=True), **values)

    def test_identity_keys_cover_formatting_and_the_north_american_twin(self):
        self.assertEqual(identity_keys('+19086420729'), ('phone:19086420729', 'phone:9086420729'))
        self.assertEqual(identity_keys('19086420729'), ('phone:19086420729', 'phone:9086420729'))
        self.assertEqual(identity_keys('(908) 642-0729'), ('phone:9086420729', 'phone:19086420729'))
        self.assertEqual(identity_keys('MChuisano@Me.com'), ('email:mchuisano@me.com',))
        self.assertEqual(identity_keys('+442071838750'), ('phone:442071838750',), 'no twin outside the NANP lengths')
        self.assertEqual(identity_keys('Bank Alert'), ())
        self.assertEqual(identity_keys(''), ())

    def test_snapshot_text_with_no_plus_joins_the_thread_instead_of_duplicating_it(self):
        # The real case: the thread key is an Apple-ID email, its recipient is "+1…", and the
        # snapshot reports the same number as "1…" without the plus.
        thread_ = self.direct('address:email:mike@me.com', '+19086420729')
        thread_['messages'] = [dict(handle='old', body='old', timestamp='2026-10-20T16:57:26', outgoing=False, read=True)]
        result = merge_inbox([thread_], [self.snap('new1', '+19086420729', '2026-10-20T20:18:09', sender_phone_norm='19086420729', read=False),
                                         self.snap('new2', '+19086420729', '2026-10-20T20:32:02', sender_phone_norm='19086420729', read=False)])
        self.assertEqual(len(result), 1, 'one conversation for one person')
        self.assertNotIn('inbox_only', result[0])
        self.assertEqual([m['handle'] for m in result[0]['messages']], ['old', 'new1', 'new2'])
        self.assertEqual(result[0]['last_ts'], '2026-10-20T20:32:02')
        self.assertEqual(result[0]['unread_count'], 2)
        self.assertEqual(result[0]['key'], 'address:email:mike@me.com', 'the backend route is untouched')

    def test_ten_and_eleven_digit_forms_and_spacing_match_each_other(self):
        for sender in ('9086420729', '19086420729', '+1 908-642-0729', '+19086420729'):
            for recipient in ('+19086420729', '9086420729', '19086420729'):
                result = merge_inbox([self.direct('t', recipient)], [self.snap('h', sender, '2026-10-20T10:00:00')])
                self.assertEqual([len(r['messages']) for r in result], [1], '%s vs %s' % (sender, recipient))

    def test_two_threads_claiming_one_number_are_never_guessed_between(self):
        a, b = self.direct('a', '+19086420729'), self.direct('b', '9086420729')
        result = merge_inbox([a, b], [self.snap('h1', '19086420729', '2026-10-20T10:00:00'), self.snap('h2', '+19086420729', '2026-10-20T10:05:00')])
        by_key = {r['key']: r for r in result}
        self.assertEqual(len(by_key['a']['messages']) + len(by_key['b']['messages']), 0, 'neither claimant gets the text')
        extra = [r for r in result if r.get('inbox_only')]
        self.assertEqual(len(extra), 1, 'both texts land together in one phone-only conversation')
        self.assertEqual(len(extra[0]['messages']), 2)

    def test_phone_only_texts_from_one_sender_share_a_conversation_whatever_the_spelling(self):
        result = merge_inbox([], [self.snap('h1', '+17175550100', '2026-10-20T10:00:00'), self.snap('h2', '7175550100', '2026-10-20T10:05:00'),
                                  self.snap('h3', '17175550100', '2026-10-20T10:06:00')])
        self.assertEqual([len(r['messages']) for r in result], [3])

    def test_groups_and_unknown_senders_are_still_left_alone(self):
        group_ = dict(key='g', name='Family', recipients=['+19086420729', '+19086420730'], is_group=True, reply_ready=True, messages=[], last_ts='')
        result = merge_inbox([group_], [self.snap('h', '19086420729', '2026-10-20T10:00:00'), self.snap('x', 'Bank Alert', '2026-10-20T10:01:00')])
        by_key = {r['key']: r for r in result}
        self.assertEqual(by_key['g']['messages'], [], 'a group never absorbs a direct text')
        self.assertEqual(len(result), 2, 'the group plus one phone-only conversation; the unknown sender is dropped')

    def test_a_sender_that_is_not_a_valid_recipient_is_still_dropped_never_made_a_route(self):
        # "(908) 642-0729" is not a valid recipient, so it must not become a conversation.
        result = merge_inbox([self.direct('t', '+19086420729')], [self.snap('h', '(908) 642-0729', '2026-10-20T10:00:00')])
        self.assertEqual([len(r['messages']) for r in result], [0])

    def test_different_numbers_are_not_merged(self):
        result = merge_inbox([self.direct('t', '+19086420729')], [self.snap('h', '19086420730', '2026-10-20T10:00:00')])
        self.assertEqual(sorted(len(r['messages']) for r in result), [0, 1])


class GuiSafetyTests(unittest.TestCase):
    """The GUI tests must never be able to open windows on a real desktop."""

    def test_only_isolated_virtual_displays_are_accepted(self):
        for display in (':99', ':100', ':102.0', ':90'):
            self.assertTrue(gui_safety.check({'DISPLAY': display})[0], display)
        for display in ('', ':0', ':0.0', ':1', ':1.0', ':10', ':89', 'host:99', 'localhost:0', ':abc'):
            self.assertFalse(gui_safety.check({'DISPLAY': display})[0], repr(display))
        self.assertFalse(gui_safety.check({})[0], 'no display at all')

    def test_the_refusal_says_what_to_do(self):
        reason = gui_safety.check({'DISPLAY': ':0'})[1]
        self.assertIn('xvfb-run', reason)
        self.assertIn('real desktop', reason)

    def test_forcing_x11_removes_the_live_wayland_session_from_view(self):
        env = {'DISPLAY': ':99', 'WAYLAND_DISPLAY': 'wayland-0', 'XDG_SESSION_TYPE': 'wayland'}
        gui_safety.force_x11(env)
        self.assertEqual(env['GDK_BACKEND'], 'x11')
        self.assertNotIn('WAYLAND_DISPLAY', env)
        self.assertEqual(env['DISPLAY'], ':99')

    def test_the_gui_test_module_skips_itself_on_a_real_looking_display(self):
        # DISPLAY=:5 and a Wayland socket that does not exist: even if the guard were
        # broken nothing could connect to a real desktop, so this is harmless to run.
        here = os.path.dirname(os.path.abspath(__file__))
        runtime = tempfile.mkdtemp()
        env = {'PATH': os.environ.get('PATH', ''), 'DISPLAY': ':5', 'WAYLAND_DISPLAY': 'no-such-socket', 'XDG_RUNTIME_DIR': runtime}
        done = subprocess.run([sys.executable, '-W', 'ignore', os.path.join(here, 'test_messages_gui.py')],
                              env=env, capture_output=True, text=True, timeout=60, cwd=here)
        self.assertEqual(done.returncode, 0, done.stderr[-400:])
        self.assertIn('skipped', done.stdout)
        self.assertIn('real desktop', done.stdout)


if __name__ == '__main__':
    unittest.main()
