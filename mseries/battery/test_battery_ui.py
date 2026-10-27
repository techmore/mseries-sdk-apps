"""GTK checks; run on a private display with temporary XDG config/data directories."""
import importlib.util
import os
from pathlib import Path
import unittest
from unittest.mock import patch


@unittest.skipUnless(os.environ.get("DISPLAY"), "Run GTK checks under xvfb-run")
class ColorSchemeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location(
            "battery_ui", Path(__file__).with_name("mseries-battery-app.py"))
        cls.ui = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.ui)
        cls.app = cls.ui.BatteryApp()
        cls.app.set_application_id("org.mseries.Battery.ThemeTest")
        with patch.object(cls.ui.Granite.Settings, "get_prefers_color_scheme",
                          return_value=cls.ui.Granite.SettingsColorScheme.DARK):
            cls.app.register()
        cls.initial_dark = cls.ui.Gtk.Settings.get_default().props.gtk_application_prefer_dark_theme

    @classmethod
    def tearDownClass(cls):
        cls.app.quit()

    def test_desktop_preference_at_startup_and_live_changes(self):
        self.assertTrue(self.initial_dark)
        for preference, expected in ((self.ui.Granite.SettingsColorScheme.NO_PREFERENCE, False),
                                     (self.ui.Granite.SettingsColorScheme.DARK, True)):
            with patch.object(self.ui.Granite.Settings, "get_prefers_color_scheme", return_value=preference):
                self.app.desktop_settings.notify("prefers-color-scheme")
                self.assertEqual(self.ui.Gtk.Settings.get_default().props.gtk_application_prefer_dark_theme,
                                 expected)


@unittest.skipUnless(os.environ.get("DISPLAY"), "Run GTK checks under xvfb-run")
class GaugeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location(
            "battery_ui_gauge", Path(__file__).with_name("mseries-battery-app.py"))
        cls.ui = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.ui)

    def test_colour_eases_from_green_to_soft_red(self):
        green, amber, red = (self.ui.level_colour(p) for p in (100, 30, 5))
        self.assertGreater(green[1], green[0])
        self.assertGreater(amber[0], amber[2])
        self.assertGreater(red[0], red[1])
        self.assertEqual(self.ui.level_colour(100), self.ui.level_colour(70))
        steps = [self.ui.level_colour(p) for p in range(100, -1, -1)]
        for a, b in zip(steps, steps[1:]):  # no jumps between neighbouring percentages
            self.assertLess(max(abs(x - y) for x, y in zip(a, b)), 0.06)

    def test_fill_height_follows_charge(self):
        def filled_rows(pct):
            surface = self.ui.cairo.ImageSurface(self.ui.cairo.FORMAT_ARGB32, 40, 64)
            gauge = self.ui.BatteryGauge(pct, False)
            gauge.draw(gauge, self.ui.cairo.Context(surface), 40, 64)
            data, stride = surface.get_data(), surface.get_stride()
            return sum(1 for y in range(64) if data[y * stride + 10 * 4 + 3] > 200
                       and data[y * stride + 10 * 4 + 1] > 120)  # opaque green-ish pixels beside the digits

        self.assertEqual(filled_rows(0), 0)
        self.assertLess(filled_rows(25), filled_rows(50))
        self.assertLess(filled_rows(50), filled_rows(100))


if __name__ == "__main__":
    unittest.main()
