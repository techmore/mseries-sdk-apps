#!/usr/bin/env python3
import json, os
from gi.repository import Gio, GLib
xml = '''<node><interface name="org.freedesktop.Notifications">
<method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/><arg type="a{sv}" direction="in"/><arg type="i" direction="in"/><arg type="u" direction="out"/></method>
<method name="CloseNotification"><arg type="u" direction="in"/></method>
<method name="GetCapabilities"><arg type="as" direction="out"/></method>
<method name="GetServerInformation"><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/></method>
</interface></node>'''
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
info = Gio.DBusNodeInfo.new_for_xml(xml)
def call(connection, sender, path, interface, method, parameters, invocation):
    if method == 'Notify':
        args = parameters.unpack()
        with open(os.environ['SIGNAL_TEST_NOTIFICATION_LOG'], 'a') as stream:
            stream.write(json.dumps({'title': args[3], 'body': args[4]}) + '\n')
        invocation.return_value(GLib.Variant('(u)', (1,)))
    elif method == 'GetCapabilities':
        invocation.return_value(GLib.Variant('(as)', (['body', 'actions'],)))
    elif method == 'GetServerInformation':
        invocation.return_value(GLib.Variant('(ssss)', ('Fixture', 'Local', '1', '1.2')))
    else:
        invocation.return_value(None)
bus.register_object('/org/freedesktop/Notifications', info.interfaces[0], call, None, None)
bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'RequestName', GLib.Variant('(su)', ('org.freedesktop.Notifications', 0)), None, Gio.DBusCallFlags.NONE, -1, None)
with open(os.environ['SIGNAL_TEST_NOTIFICATION_LOG'] + '.ready', 'w') as stream:
    stream.write('ready')
GLib.MainLoop().run()
