import json
from pathlib import Path
import tempfile
import unittest
from message_pins import SCHEMA, read_snapshot, validate_snapshot, suggested_matches, matched_state, ordered_keys, effective_choices


def snapshot(ids=None):
    return dict(schema=SCHEMA, version=1, source='macos-imcore', captured_at='2026-10-20T16:00:00+00:00',
                pins=[dict(apple_identifier=i) for i in (ids or ['+15550100123', 'opaque-group'])])


def threads():
    return dict(alex=dict(key='alex', name='Alex', recipients=['+15550100123'], last_ts='2026-10-20'),
                group=dict(key='group', name='opaque-group', is_group=True, recipients=['+15550100123', '+15550100124'], last_ts='2026-10-19'),
                newer=dict(key='newer', name='Recent', recipients=['+15550100125'], last_ts='2026-10-21'))


class PinTests(unittest.TestCase):
    def test_exact_individual_matches_but_group_name_is_not_identity(self):
        self.assertEqual(suggested_matches(snapshot(), threads()), ['alex', None])

    def test_duplicate_individual_threads_require_explicit_selection(self):
        t = threads();t['other'] = dict(t['alex'], key='other')
        self.assertEqual(suggested_matches(snapshot(), t), [None, None])

    def test_explicit_group_choice_preserves_apple_order(self):
        t = threads();state = matched_state(snapshot(), ['group', 'alex'], t)
        self.assertEqual(ordered_keys(t, state), ['group', 'alex', 'newer'])

    def test_unresolved_pins_survive_for_later_review(self):
        state = matched_state(snapshot(), ['alex', None], threads())
        self.assertEqual(len(state['snapshot']['pins']), 2)
        self.assertEqual(state['choices'], ['alex', None])

    def test_duplicate_and_disappeared_choices_rejected(self):
        for choices in (['alex', 'alex'], ['alex', 'missing']):
            with self.assertRaises(ValueError): matched_state(snapshot(), choices, threads())

    def test_missing_visible_thread_does_not_change_routing(self):
        t = threads();state = matched_state(snapshot(), ['alex', 'group'], t);del t['alex']
        self.assertEqual(ordered_keys(t, state), ['group', 'newer'])

    def test_bad_and_future_exports_rejected(self):
        for change in [dict(schema='other'), dict(captured_at='2099-01-01T00:00:00Z'), dict(captured_at='2026-10-20'), dict(pins=[dict(apple_identifier='x\nTEL:other')]), dict(pins=[dict(apple_identifier='same')]*2)]:
            value = snapshot();value.update(change)
            with self.assertRaises(ValueError): validate_snapshot(value)

    def test_bounded_import_discards_unknown_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'pins.json'
            value = snapshot();value['metadata'] = {'unneeded':'not retained'}
            path.write_text(json.dumps(value));self.assertNotIn('metadata', read_snapshot(path))
            path.write_bytes(b'x'*131073)
            with self.assertRaises(ValueError): read_snapshot(path)

    def test_verified_empty_snapshot_is_distinct_from_unavailable(self):
        value = snapshot();value['pins'] = []
        self.assertEqual(matched_state(value, [], threads())['choices'], [])
        with self.assertRaises(ValueError): validate_snapshot({'status':'unavailable'})

    def test_reimport_keeps_explicit_group_match_when_apple_reorders(self):
        state = matched_state(snapshot(), ['alex', 'group'], threads())
        reordered = snapshot(['opaque-group', '+15550100123'])
        self.assertEqual(suggested_matches(reordered, threads(), state), ['group', 'alex'])

    def test_phone_only_individual_promotes_without_group_guessing(self):
        state = dict(snapshot=snapshot(), choices=['inbox:+15550100123', 'missing-group'])
        self.assertEqual(effective_choices(threads(), state), ['alex', 'missing-group'])
        state['choices'][0] = None
        self.assertEqual(effective_choices(threads(), state), [None, 'missing-group'])

    def test_cache_source_preserves_provenance_and_apple_modification_time(self):
        value = snapshot();value.update(source='macos-pin-cache', source_modified_at='2026-10-19T16:00:00Z')
        parsed = validate_snapshot(value)
        self.assertEqual(parsed['source'], 'macos-pin-cache')
        self.assertEqual(parsed['source_modified_at'], '2026-10-19T16:00:00+00:00')
        value['source_modified_at'] = '2099-01-01T00:00:00Z'
        with self.assertRaises(ValueError): validate_snapshot(value)


if __name__ == '__main__': unittest.main()
