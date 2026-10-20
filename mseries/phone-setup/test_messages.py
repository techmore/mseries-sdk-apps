import unittest
from messages_model import ConversationState, merge_inbox, contact_index, resolve_contact_names


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


if __name__ == '__main__':
    unittest.main()
