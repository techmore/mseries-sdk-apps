import unittest
from datetime import datetime, timezone
from code_offers import Offers, code_field

NOW = datetime(2026, 10, 20, 16, 0, tzinfo=timezone.utc).timestamp()


def message(body='Your verification code is 123456.', age=10, handle='one'):
    return dict(handle=handle, body=body, timestamp=datetime.fromtimestamp(NOW-age, timezone.utc).isoformat(), sender='Test service')


class CodesTests(unittest.TestCase):
    def setUp(self):
        self.offers = Offers(lambda: NOW)

    def test_code_is_transient_and_not_in_repr(self):
        self.offers.update([message()])
        offer = self.offers.available()[0]
        self.assertEqual(offer.code, '123456')
        self.assertNotIn('123456', repr(offer))

    def test_old_future_and_missing_time_are_rejected(self):
        self.offers.update([message(age=301), message(age=-1, handle='future'), dict(handle='bad', body='OTP 123456')])
        self.assertEqual(self.offers.available(), [])

    def test_refresh_preserves_identity_and_use_consumes_it(self):
        self.offers.update([message()]);first = self.offers.available()[0]
        self.offers.update([message()]);self.assertEqual(self.offers.available()[0].id, first.id)
        self.offers.consume(first.id);self.offers.update([message()])
        self.assertEqual(self.offers.available(), [])

    def test_origin_bound_message_cannot_fill_wrong_site(self):
        self.offers.update([message('Your verification code is 123456.\n@example.com #123456')])
        self.assertEqual(self.offers.available('https://other.example'), [])
        self.assertEqual(self.offers.available(), [])
        self.assertEqual(len(self.offers.available('https://example.com/login')), 1)
        self.assertEqual(self.offers.available('http://example.com'), [])

    def test_embedded_domain_and_mismatched_binding_not_guessed(self):
        for body in ['Your verification code is 123456.\n@example.com #999999', 'Your verification code is 123456.\n@example.com #123456 @embedded.example']:
            self.offers.update([message(body)])
            self.assertEqual(self.offers.available('https://example.com'), [])

    def test_orders_promotions_and_unrelated_numbers_not_offered(self):
        for body in ['Your order number is 123456.', 'Save 1234 points today.', 'Our door code is 123456.']:
            self.offers.update([message(body)])
            self.assertEqual(self.offers.available(), [])

    def test_fields_require_code_context(self):
        self.assertTrue(code_field('Verification code', {}))
        self.assertTrue(code_field('', {'autocomplete':'one-time-code'}))
        self.assertFalse(code_field('Password', {}))
        self.assertFalse(code_field('Message', {}))
        self.assertFalse(code_field('Code', {}))
        self.assertTrue(code_field('Code', {}, allow_plain=True))

    def test_live_expiry_checked_again_at_use(self):
        now = [NOW]
        offers = Offers(lambda: now[0]);offers.update([message()]);identity = offers.available()[0].id
        now[0] += 301
        self.assertIsNone(offers.get(identity))

    def test_shortcut_aliases_and_modifier_order_do_not_collide(self):
        from code_shortcut import accelerator, choose
        self.assertEqual(accelerator('<Shift><Super>v'), accelerator('<Super><Shift>v'))
        self.assertEqual(accelerator('<Primary><Alt>o'), accelerator('<Control><Alt>o'))
        self.assertEqual(choose(['<Shift><Super>v']), '<Super><Shift>o')
        self.assertEqual(choose(['<Shift><Super>v','<Shift><Super>o','<Primary><Alt>o']), '')


if __name__ == '__main__':
    unittest.main()
