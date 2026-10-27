#!/usr/bin/python3
import gi
gi.require_version("Gtk", "4.0")
from gi.repository import Gtk
app = Gtk.Application(application_id="io.github.techmore.BenchNoQuit")
def activate(instance):
    window = Gtk.ApplicationWindow(application=instance, title="No quit fixture")
    window.set_child(Gtk.Label(label="Ctrl+Q intentionally unsupported"))
    window.present()
app.connect("activate", activate)
raise SystemExit(app.run([]))
