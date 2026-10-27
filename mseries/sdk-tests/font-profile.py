#!/usr/bin/env python3
"""Assert the real GTK font configuration used by a private render profile."""
import os
import gi
gi.require_version('Gtk', '4.0')
from gi.repository import Gtk
Gtk.init()
expected = 'Sans ' + os.environ['MSDK_MATRIX_POINTS']
actual = Gtk.Settings.get_default().get_property('gtk-font-name')
assert actual == expected, (actual, expected)
print('GTK font profile:', actual)
