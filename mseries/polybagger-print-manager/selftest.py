"""Printer-selection checks with fake USB devices and CUPS queues (no printer, no CUPS needed)."""
import importlib.util
import sys
from pathlib import Path

spec = importlib.util.spec_from_file_location("pbpm", Path(__file__).with_name("polybagger_print_manager.py"))
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
m.default_queue_name = lambda: None


def dev(serial):
    d = {"bus": "1-1", "vendor_id": "0a5f", "product_id": "00a9", "product": "ZTC 110Xi4-203dpi ZPL",
         "serial": serial, "manufacturer": "Zebra Technologies"}
    d["uri"] = m.usb_uri(d); d["identity"] = f"0a5f:00a9:{serial}"
    return d


def q(name, serial):
    return {"name": name, "uri": m.usb_uri(dev(serial))}


old, new = "JJK245392", "JJL100932"
queues = [q("ZTC", old), q("ZTC-2", new)]

# saved choice is the unplugged bagger; the plugged-in one must win
t = m.printer_targets([dev(new)], queues)
assert m.choose_target(t, "ZTC")["queue"] == "ZTC-2"
# saved choice is plugged in: keep it
t = m.printer_targets([dev(old), dev(new)], queues)
assert m.choose_target(t, "ZTC")["queue"] == "ZTC"
# explicit --printer stays strict even when unplugged
t = m.printer_targets([dev(new)], queues)
assert m.choose_target(t, "ZTC", strict=True)["queue"] == "ZTC"
assert m.find_target(t, old)["queue"] == "ZTC"
# new serial with no queue yet: offered for setup, and chosen over an unplugged saved queue
t = m.printer_targets([dev("NEW123")], [q("ZTC", old)])
c = m.choose_target(t, "ZTC")
assert c["needs_setup"] and c["device"]["serial"] == "NEW123"
# a connected device with a queue beats a second connected device needing setup
t = m.printer_targets([dev("NEW123"), dev(new)], queues)
assert m.choose_target(t, None)["queue"] == "ZTC-2"
# nothing plugged in: keep the saved queue so its state can be shown
t = m.printer_targets([], queues)
c = m.choose_target(t, "ZTC")
assert c["queue"] == "ZTC" and not c["connected"]
print("selftest ok")
sys.exit(0)
