#!/usr/bin/env python3
"""PolyBagger T1000 Print Manager: load labels onto the T-1000's inline Zebra printer and verify each step.

The printer (Zebra ZTC 110Xi4 engine in an Advanced Poly TI-1000Z frame) prints onto the bag film only when
the bagger cycles. Sending ZPL *loads* a label; the bagger's Printer Settings screen then changes from
"No Label" to "New Label" (or "StoredLabel" after a recall), and each Manual Cycle prints one label.

GUI (elementary OS, GTK 4 + Granite 7):   ./polybagger_print_manager.py
Terminal:
  ./polybagger_print_manager.py --status                       all printer targets + selected CUPS state
  ./polybagger_print_manager.py --send FILE.zpl [--qty N] [--yes]  load a label for N bags (^PQ) and verify, exit 0/1/2
  ./polybagger_print_manager.py --send FILE.zpl --printer QUEUE_OR_SERIAL
  ./polybagger_print_manager.py --watch                        print a line each time the printer is plugged/unplugged
  ./polybagger_print_manager.py --sync [--pull]                refresh the offline label store (works with no network)
  ./polybagger_print_manager.py --cancel                       cancel your stuck jobs
  ./polybagger_print_manager.py --setup --printer SERIAL       create a per-device raw CUPS queue (asks for admin auth)
Add --json to --status / --send for machine-readable output.
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
from pathlib import Path

USB_PRINTER_CLASS = "07"
MANAGED_QUEUE_PREFIX = "PolyBagger"
DATA_DIR = Path.home() / ".local/share/polybagger-print-manager"
LOG_PATH = DATA_DIR / "log.jsonl"
STORE = DATA_DIR / "labels"  # offline copy of the label library; printing never needs the network
THRIL_REPO = Path.home() / "Projects/thril"
SOURCES = [THRIL_REPO / "poly-bagger/UPC Labels"]
PREFERRED = ["CCSBLACK_recall_flash.zpl", "ccs_black_label.zpl", "CCSBLACK_store_flash.zpl"]
JOB_TIMEOUT = 60

TEST_LABEL = "^XA\n^PW700\n^LL320\n^FO40,40^A0N,55,55^FDPRINTER TEST^FS\n" \
             "^FO40,125^A0N,40,40^FD{stamp}^FS\n^FO40,205^GB620,70,5^FS\n^XZ\n"


def run(cmd, timeout=15):
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return r.returncode, (r.stdout + r.stderr).strip()
    except (subprocess.TimeoutExpired, FileNotFoundError) as e:
        return 127, str(e)


# ---------------------------------------------------------------- detection

def usb_printers():
    """USB printer-class devices currently on the bus, read from sysfs."""
    found = []
    for d in Path("/sys/bus/usb/devices").glob("*"):
        try:
            read = lambda n: (d / n).read_text().strip() if (d / n).exists() else ""
            vendor_id, product_id = read("idVendor").lower(), read("idProduct").lower()
            if not vendor_id:
                continue
            interfaces = d.glob(f"{d.name}:*/bInterfaceClass")
            printer_class = any(p.read_text().strip().lower() == USB_PRINTER_CLASS for p in interfaces)
            # Some Zebra firmware exposes a vendor-specific class instead of the USB printer class.
            if not printer_class and vendor_id != "0a5f":
                continue
            device = {"bus": d.name, "vendor_id": vendor_id, "product_id": product_id,
                      "product": read("product") or "USB printer",
                      "serial": read("serial"), "manufacturer": read("manufacturer")}
            device["uri"] = usb_uri(device)
            device["identity"] = ":".join((vendor_id, product_id, device["serial"] or d.name))
            found.append(device)
        except OSError:
            pass
    return sorted(found, key=lambda d: (d["serial"], d["product"], d["bus"]))


def usb_uri(dev):
    q = urllib.parse.quote
    uri = f"usb://{q(dev['manufacturer'] or 'USB Printer')}/{q(dev['product'])}"
    return uri + (f"?serial={q(dev['serial'])}" if dev["serial"] else "")


def cups_queues():
    """Return every configured CUPS destination and its device URI."""
    _, out = run(["lpstat", "-v"])
    return [{"name": name, "uri": uri} for name, uri in re.findall(r"^device for (\S+): (.+)$", out, re.M)]


def matching_usb_device(uri, devices):
    """Match a CUPS USB URI to one currently connected device; ambiguous matches fail closed."""
    if not uri.lower().startswith("usb://"):
        return None
    parts = urllib.parse.urlsplit(uri)
    query = urllib.parse.parse_qs(parts.query)
    serial = query.get("serial", [""])[0]
    if serial:
        matches = [dev for dev in devices if dev.get("serial") == serial]
        if len(matches) > 1:
            uri_path = (urllib.parse.unquote(parts.netloc), urllib.parse.unquote(parts.path))
            matches = [dev for dev in matches
                       if (urllib.parse.unquote(urllib.parse.urlsplit(dev["uri"]).netloc),
                           urllib.parse.unquote(urllib.parse.urlsplit(dev["uri"]).path)) == uri_path]
    else:
        uri_base = urllib.parse.unquote(uri).split("?", 1)[0].rstrip("/")
        matches = [dev for dev in devices
                   if urllib.parse.unquote(dev["uri"]).split("?", 1)[0].rstrip("/") == uri_base]
    return matches[0] if len(matches) == 1 else None


def printer_targets(devices=None, queues=None):
    """Configured CUPS destinations plus detected USB printers that need a queue."""
    devices = usb_printers() if devices is None else devices
    queues = cups_queues() if queues is None else queues
    targets = []
    matched_ids = set()
    for queue in queues:
        uri = queue["uri"]
        is_usb = uri.lower().startswith("usb://")
        device = matching_usb_device(uri, devices) if is_usb else None
        serial = urllib.parse.parse_qs(urllib.parse.urlsplit(uri).query).get("serial", [""])[0]
        if device:
            matched_ids.add(device["identity"])
            detail = f"{device['product']} · {device['serial'] or device['bus']}"
            display = f"{queue['name']} — {detail}"
        elif is_usb:
            model = urllib.parse.unquote(urllib.parse.urlsplit(uri).path.rsplit("/", 1)[-1])
            detail = " · ".join(p for p in (model, urllib.parse.unquote(serial)) if p)
            display = f"{queue['name']} — USB {detail}"
        else:
            display = f"{queue['name']} — Network / CUPS"
        targets.append({"id": queue["name"], "queue": queue["name"], "uri": uri,
                        "transport": "usb" if is_usb else "cups", "device": device,
                        "connected": bool(device) if is_usb else True,
                        "needs_setup": False, "display": display})
    for device in devices:
        if device["identity"] in matched_ids:
            continue
        targets.append({"id": f"usb:{device['identity']}", "queue": None, "uri": device["uri"],
                        "transport": "usb", "device": device, "connected": True,
                        "needs_setup": True,
                        "display": f"{device['product']} · {device['serial'] or device['bus']} (Set up)"})
    return targets


def default_queue_name():
    _, out = run(["lpstat", "-d"])
    match = re.search(r"system default destination: (\S+)", out)
    return match.group(1) if match else None


def find_target(targets, preferred):
    """Resolve a queue, target ID, USB identity, or unique device serial."""
    exact = [t for t in targets if preferred in (t["id"], t.get("queue"))]
    if len(exact) == 1:
        return exact[0]
    identities = [t for t in targets if t.get("device") and preferred == t["device"]["identity"]]
    if len(identities) == 1:
        return identities[0]
    serials = [t for t in targets if t.get("device") and preferred == t["device"].get("serial")]
    return serials[0] if len(serials) == 1 else None


def choose_target(targets, preferred=None):
    if preferred:
        target = find_target(targets, preferred)
        if target:
            return target
    connected_usb = [t for t in targets if t["transport"] == "usb" and t["connected"]]
    if connected_usb:
        return connected_usb[0]
    default = default_queue_name()
    for target in targets:
        if target.get("queue") == default and target["connected"]:
            return target
    return targets[0] if targets else None


def queue_state(name):
    """('idle'|'processing'|'disabled'|'unknown', [alert reasons])."""
    _, out = run(["lpstat", "-l", "-p", name])
    m = re.search(r"printer \S+ (?:is )?(idle|now printing|disabled)", out)
    state = {"idle": "idle", "now printing": "processing", "disabled": "disabled"}.get(m.group(1) if m else "", "unknown")
    alerts = re.search(r"Alerts?: (.*)", out)
    reasons = [] if not alerts or alerts.group(1).strip() == "none" else alerts.group(1).split()
    if "Waiting for printer to become available" in out:
        reasons.append("waiting-for-printer")
    return state, reasons


def status(preferred=None):
    devices = usb_printers()
    targets = printer_targets(devices)
    explicit = preferred is not None
    preferred = preferred or load_settings().get("printer_target")
    target = choose_target(targets, preferred)
    if explicit and not find_target(targets, preferred):
        target = None
    name = target.get("queue") if target else None
    uri = target.get("uri") if target else None
    qstate, reasons = queue_state(name) if name else ("missing", [])
    connected = bool(target and target["connected"])
    uri_ok = bool(target and (target["transport"] != "usb" or connected))
    ready = bool(target and name and connected and qstate in ("idle", "processing") and not reasons)
    return {"usb": devices, "targets": targets,
            "selected_target": target["id"] if target else None, "target": target,
            "device": target.get("device") if target else None, "connected": connected,
            "queue": name, "uri": uri, "queue_state": qstate,
            "reasons": reasons, "uri_matches_device": uri_ok, "ready": ready}


def managed_queue_name(dev):
    identity = dev.get("serial") or f"{dev.get('vendor_id', 'usb')}-{dev.get('product_id', dev.get('bus', 'printer'))}-{dev.get('bus', '')}"
    slug = re.sub(r"[^A-Za-z0-9_.-]+", "-", identity).strip("-.")[:72] or "USB"
    name = f"{MANAGED_QUEUE_PREFIX}-{slug}"
    for queue in cups_queues():
        if queue["name"] == name and urllib.parse.unquote(queue["uri"]) != urllib.parse.unquote(usb_uri(dev)):
            digest = hashlib.sha256(usb_uri(dev).encode()).hexdigest()[:8]
            name = f"{name[:110]}-{digest}"
            break
    return name


def setup_queue(dev):
    name = managed_queue_name(dev)
    if not dev.get("serial"):
        same_uri = [d for d in usb_printers()
                    if urllib.parse.unquote(d["uri"]).split("?", 1)[0] ==
                    urllib.parse.unquote(usb_uri(dev)).split("?", 1)[0]]
        if len(same_uri) > 1:
            return 1, "These identical USB printers have no serial number, so CUPS cannot route to one safely.", name
    cmd = ["pkexec", "lpadmin", "-p", name, "-E", "-v", usb_uri(dev), "-m", "raw",
           "-o", "printer-is-shared=false", "-o", "printer-error-policy=retry-job"]
    rc, out = run(cmd, timeout=120)
    return rc, out, name


# ---------------------------------------------------------------- sending

def check_zpl(path):
    """(ok, warnings) for a ZPL file before sending."""
    warns = []
    try:
        data = Path(path).read_text(errors="replace")
    except OSError as e:
        return False, [f"cannot read file: {e}"]
    if "^XA" not in data or "^XZ" not in data:
        return False, ["not ZPL: missing ^XA ... ^XZ"]
    if re.search(r"\^PON|\^FWN", data):
        warns.append("contains ^PON/^FWN. The T-1000Z printer rejected these commands; verify support on the selected printer.")
    return True, warns


STUCK_HINT = ("The printer is not taking data. Check that it is powered on and online; on a TI-1000Z, set Printer "
              "to ON and Run mode (not Setup). If it stays stuck, reconnect or power-cycle it, then cancel stuck jobs.")

MACHINE_FLOW = [
    "Printer Settings on the bagger: printer ON, Run mode (not Setup).",
    "Set Labels to print to the bags in this run (for example 1000). It is sent as ^PQ, the label's Quantity.",
    "Send the label. The screen changes from No Label to New Label, or StoredLabel after a recall.",
    "Press Manual Cycle: one bag through, one label printed.",
    "Check Reprint: ReprintBuffer or Reprint ON keeps printing the same label every cycle.",
    "Each bag cycle uses one label. When the quantity is used up the label clears (back to No Label).",
    "Clear Labels unloads the current label (back to No Label). Send or recall it again.",
]

KNOWN_ISSUES = [
    "The app sends raw ZPL. Select only a printer queue whose device supports ZPL; network printers must first be added in System Settings.",
    "USB printer-class devices are discovered every second. Queues match by serial or a unique USB URI; identical devices without serials cannot be distinguished safely.",
    "The library's label files carry no ^PQ. The app adds the chosen quantity to a temporary copy; printer support for ^PQ varies.",
    "CUPS \"completed\" means the printer received the data. It does not mean a label printed: that needs a bag cycle.",
    "The printer can stop taking data (jobs sit at \"waiting for printer\") or drop off USB. A printer reboot has cleared it.",
    "After a drop-off CUPS may disable the queue. Re-enabling needs admin: sudo cupsenable <queue>.",
    "Sending the same label twice can leave it printing on every cycle if Reprint is on. Check the bagger screen.",
    "Clear Labels on the bagger unloads the label. Stored flash formats (E:) are not erased by it, per the TI-1000Z manual.",
    "Existing CUPS queues can use any driver, but this app sends raw ZPL and does not convert languages. New USB queues are private raw queues.",
    "The printer hardware has its own intermittent faults (see docs/polybagger-t1000-printer-link-troubleshooting.md).",
]


def expectation_kind(path):
    try:
        return expectation(path)[0]
    except OSError:
        return "label"


def kind_of(path):
    try:
        d = Path(path).read_text(errors="replace")
    except OSError:
        return "Unreadable"
    if "^XF" in d: return "Recalls from printer flash"
    if "^DF" in d: return "Stores to printer flash"
    return "Loads a label"


def expectation(path):
    """(kind, what to verify at the printer after it receives the job)."""
    data = Path(path).read_text(errors="replace")
    if "^XF" in data:
        return "recall", "Recalls the stored label format from printer flash."
    if "^DF" in data and "^PQ" not in data:
        return "store", "Saves the label to printer flash. No label prints; send its recall file next."
    return "label", "Sends label data to the printer. Confirm it loaded or printed."


def send_and_verify(path, queue, progress=lambda msg: None):
    """lp -o raw, then watch CUPS until the job is completed, cancelled or stuck."""
    t0 = time.time()
    rc, out = run(["lp", "-d", queue, "-n", "1", "-o", "raw", "-t", Path(path).name, str(path)], timeout=30)
    m = re.search(r"request id is (\S+)", out)
    if rc != 0 or not m:
        return {"ok": False, "stage": "submit", "detail": out or "lp failed"}
    job = m.group(1)
    progress(f"Submitted {job}")
    seen_active = False
    gone = 0
    while time.time() - t0 < JOB_TIMEOUT:
        _, active = run(["lpstat", "-W", "not-completed", "-o", queue])
        if job in active:
            seen_active, gone = True, 0
            state, reasons = queue_state(queue)
            progress(f"{job} in queue, printer {state}" + (f" ({', '.join(reasons)})" if reasons else ""))
        else:
            _, done = run(["lpstat", "-W", "completed", "-o", queue])
            if job in done:
                return {"ok": True, "stage": "completed", "job": job,
                        "seconds": round(time.time() - t0, 1), "detail": "Delivered: CUPS reports the printer received it"}
            gone += 1
            if gone >= 3:
                return {"ok": False, "stage": "vanished", "job": job,
                        "detail": "job left the queue without completing (cancelled or aborted)"}
        time.sleep(1)
    state, reasons = queue_state(queue)
    _, jobline = run(["lpstat", "-W", "not-completed", "-o", queue])
    return {"ok": False, "stage": "timeout", "job": job,
            "detail": f"Not delivered after {JOB_TIMEOUT}s (printer {state}, {', '.join(reasons) or 'no alerts'}). {STUCK_HINT}"}


def with_quantity(path, qty):
    """Path to send: a temporary copy with ^PQ<qty> (the label Quantity) for printable labels.
    Store-only files (^DF) are returned unchanged; they print nothing."""
    data = Path(path).read_text(errors="replace")
    kind, _ = expectation(path)
    if kind == "store" or qty is None:
        return Path(path), False
    data = re.sub(r"\^PQ[0-9]+(,[0-9]*)*(,[YN])*", "", data)
    i = data.rindex("^XZ")
    data = data[:i] + f"^PQ{int(qty)}\n" + data[i:]
    fd, tmp = tempfile.mkstemp(prefix=Path(path).stem + "-", suffix=".zpl")
    os.write(fd, data.encode()); os.close(fd)
    return Path(tmp), True


def resume_queue(queue):
    """Re-enable a paused queue and set it to retry (not pause) when the printer is power-cycled. Asks for admin auth."""
    rc, out = run(["pkexec", "cupsenable", queue], timeout=180)
    if rc:
        return rc, out
    return run(["pkexec", "lpadmin", "-p", queue, "-o", "printer-error-policy=retry-job"], timeout=180)


def my_stuck_jobs(queue):
    _, out = run(["lpstat", "-W", "not-completed", "-o", queue])
    return re.findall(r"^(\S+-\d+)\s", out, re.M)


def cancel_jobs(queue):
    """Cancel queued jobs one by one (cancel -a needs admin; your own jobs do not)."""
    done, failed = [], []
    for job in my_stuck_jobs(queue):
        (done if run(["cancel", job])[0] == 0 else failed).append(job)
    return done, failed


SETTINGS = DATA_DIR / "settings.json"


def load_settings():
    try:
        return json.loads(SETTINGS.read_text())
    except (OSError, ValueError):
        return {}


def save_settings(**kv):
    try:
        DATA_DIR.mkdir(parents=True, exist_ok=True)
        SETTINGS.write_text(json.dumps({**load_settings(), **kv}))
    except OSError:
        pass


def log_result(entry):
    try:
        LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
        with LOG_PATH.open("a") as f:
            f.write(json.dumps({"time": time.strftime("%Y-%m-%d %H:%M:%S"), **entry}) + "\n")
    except OSError:
        pass


def sync_library(pull=False):
    """Copy new/changed .zpl labels from the thril checkout into the local store. Returns (copied, total, note)."""
    import hashlib, shutil
    note = ""
    if pull and (THRIL_REPO / ".git").is_dir():
        rc, out = run(["git", "-C", str(THRIL_REPO), "pull", "--ff-only", "-q"], timeout=20)
        note = "git pull ok" if rc == 0 else f"git pull failed (offline?): {out.splitlines()[-1] if out else rc}"
    STORE.mkdir(parents=True, exist_ok=True)
    sha = lambda f: hashlib.sha256(f.read_bytes()).hexdigest()
    copied = 0
    for src in SOURCES:
        for f in src.glob("*.zpl") if src.is_dir() else []:
            dst = STORE / f.name
            if not dst.exists() or sha(dst) != sha(f):
                shutil.copy2(f, dst); copied += 1
    files = sorted(STORE.glob("*.zpl"))
    (STORE / "manifest.json").write_text(json.dumps(
        {"synced_at": time.strftime("%Y-%m-%d %H:%M:%S"), "sources": [str(x) for x in SOURCES],
         "files": {f.name: sha(f) for f in files}}, indent=1))
    return copied, len(files), note


def label_files():
    try:
        sync_library()
    except OSError:
        pass
    if not STORE.is_dir():
        return []
    files = sorted(p for p in STORE.glob("*.zpl"))
    pref = [p for n in PREFERRED for p in files if p.name == n]
    return pref + [p for p in files if p not in pref]


def label_preview(path):
    """((width, height, rgb_bytes)|None, caption) rendered from the ^GFA graphics in a ZPL file.
    A recall file (^XF) is resolved to the sibling file that stores that format (^DF)."""
    path = Path(path)
    data = path.read_text(errors="replace")
    note = ""
    m = re.search(r"\^XF(\S+?)\^FS", data)
    if m:
        name = m.group(1)
        src = next((q for q in path.parent.glob("*.zpl") if f"^DF{name}" in q.read_text(errors="replace")), None)
        if not src:
            return None, f"Recalls {name} from printer flash (no stored copy found next to this file)"
        data, note = src.read_text(errors="replace"), f"Recalls {name} from printer flash. Stored copy: {src.name}. "
    pw = re.search(r"\^PW(\d+)", data); ll = re.search(r"\^LL(\d+)", data)
    gfx = [(int(x), int(y), int(bpr), h) for x, y, bpr, h in
           re.findall(r"\^FO(\d+),(\d+)\^GFA,\d+,\d+,(\d+),([0-9A-Fa-f]+)", data)]
    if not gfx:
        return None, note + "No bitmap graphic in this file (text/barcode commands only)."
    lut = [b"".join(b"\x00\x00\x00" if b >> (7 - i) & 1 else b"\xff\xff\xff" for i in range(8)) for b in range(256)]
    W = int(pw.group(1)) if pw else max(x + bpr * 8 for x, _, bpr, _ in gfx)
    H = int(ll.group(1)) if ll else max(y + len(h) // 2 // bpr for x, y, bpr, h in gfx)
    canvas = bytearray(b"\xff" * (W * H * 3))
    for x, y, bpr, hexdata in gfx:
        raw = bytes.fromhex(hexdata[: len(hexdata) // 2 * 2])
        for r in range(len(raw) // bpr):
            if y + r >= H: break
            row = b"".join(lut[b] for b in raw[r * bpr:(r + 1) * bpr])[: max(0, (W - x)) * 3]
            canvas[((y + r) * W + x) * 3: ((y + r) * W + x) * 3 + len(row)] = row
    return (W, H, bytes(canvas)), f"{note}{W}x{H} dots, {W/203:.1f} x {H/203:.1f} in at 203 dpi"


# ---------------------------------------------------------------- CLI

def describe(st):
    target = st.get("target")
    if not target:
        if st.get("targets"):
            return "No matching printer selected. Available: " + "; ".join(t["display"] for t in st["targets"])
        return "No CUPS printers or USB printers found. Connect a printer or add a CUPS queue.\nNOT READY"
    else:
        lines = [f"Selected: {target['display']}"]
    others = [t["display"] for t in st.get("targets", []) if not target or t["id"] != target["id"]]
    if others:
        lines.append("Other printers: " + "; ".join(others))
    if target["needs_setup"]:
        serial = target["device"].get("serial") or target["device"]["identity"]
        lines.append(f"CUPS: no queue. Run: ./polybagger_print_manager.py --setup --printer {serial}")
    elif not st["connected"]:
        lines.append(f"USB printer is disconnected; queue '{st['queue']}' is retained for replug.")
    else:
        lines.append(f"CUPS: queue '{st['queue']}' is {st['queue_state']}" +
                     (f", alerts: {' '.join(st['reasons'])}" if st["reasons"] else ""))
    return "\n".join(lines + (["READY"] if st["ready"] else ["NOT READY"]))


def cli(args):
    if args.cancel:
        st = status(args.printer); done, failed = cancel_jobs(st["queue"]) if st["queue"] else ([], [])
        print(f"cancelled: {' '.join(done) or 'none'}" + (f"; failed: {' '.join(failed)}" if failed else "")); return 1 if failed else 0
    if args.setup:
        devices = usb_printers()
        st = status(args.printer)
        target = st.get("target")
        dev = target.get("device") if target else None
        if not args.printer:
            if len(devices) == 1:
                dev = devices[0]
            else:
                print("Choose one connected USB printer with --printer SERIAL. Available: " +
                      (", ".join(d.get("serial") or d["identity"] for d in devices) or "none"))
                return 2
        if not dev:
            print("Select a connected USB printer by serial with --printer. Available: " +
                  (", ".join(d.get("serial") or d["identity"] for d in devices) or "none"))
            return 2
        rc, out, name = setup_queue(dev)
        print(out or f"CUPS queue {name} created for {dev['product']} ({dev.get('serial') or dev['bus']})")
        return 0 if rc == 0 else 1
    if args.sync:
        c, n, note = sync_library(pull=args.pull); print(f"{n} labels in {STORE} ({c} updated) {note}".strip()); return 0
    if args.watch:
        prev = None
        while True:
            cur = tuple(d["identity"] for d in usb_printers())
            if cur != prev:
                before, after = set(prev or ()), set(cur)
                by_id = {d["identity"]: d for d in usb_printers()}
                for identity in sorted(after - before):
                    d = by_id.get(identity, {})
                    print(time.strftime("%H:%M:%S"), f"PRINTER PLUGGED IN · {d.get('product', identity)} · {d.get('serial') or identity}", flush=True)
                for identity in sorted(before - after):
                    print(time.strftime("%H:%M:%S"), f"PRINTER UNPLUGGED · {identity}", flush=True)
                prev = cur
            time.sleep(1)
    st = status(args.printer)
    if args.status or not args.send:
        print(json.dumps(st, indent=2) if args.json else describe(st))
        return 0 if st["ready"] else 2
    ok, warns = check_zpl(args.send)
    if not ok:
        print("ERROR:", warns[0]); return 1
    if not st["ready"]:
        print(describe(st)); return 2
    for w in warns:
        print("WARNING:", w, file=sys.stderr)
    job_path, temp = with_quantity(args.send, args.qty)
    try:
        res = send_and_verify(job_path, st["queue"], progress=lambda m: print(" ", m, file=sys.stderr))
    finally:
        if temp: os.unlink(job_path)
    res.update(quantity=args.qty, queue=st["queue"], printer_target=st["selected_target"])
    kind, hint = expectation(args.send)
    res.update(file=str(args.send), expect=hint)
    if res["ok"] and kind != "store" and not args.yes and sys.stdin.isatty():
        res["bagger_loaded"] = input(f"{hint} Does it? [y/N] ").strip().lower().startswith("y")
        if res["bagger_loaded"]:
            res["labels_per_cycle"] = input("Press Manual Cycle once. How many labels printed? [0/1/2+] ").strip() or "?"
    elif res["ok"]:
        print(hint, file=sys.stderr)
    log_result(res)
    print(json.dumps(res, indent=2) if args.json else
          ("PASS" if res["ok"] else "FAIL") + f": {res['detail']}" + (f" ({res['job']}, {res['seconds']}s)" if res["ok"] else ""))
    return 0 if res["ok"] else 1


# ---------------------------------------------------------------- GUI

CSS = """
window.pbpm label { font-size: 15px; }
window.pbpm label.pbpm-title { font-size: 28px; font-weight: 700; }
window.pbpm label.pbpm-row-title { font-size: 16px; font-weight: 600; }
window.pbpm label.pbpm-sub { font-size: 13px; opacity: 0.75; }
window.pbpm .pbpm-sidebar row { padding: 10px 14px; }
window.pbpm .pbpm-card { padding: 18px; }
window.pbpm button.pbpm-send { font-size: 17px; font-weight: 600; padding: 10px 30px; }
window.pbpm .pbpm-help label { font-size: 14px; }
window.pbpm .pbpm-statusbar { padding: 8px 14px; border-bottom: 1px solid alpha(currentColor, 0.12); }
window.pbpm .pbpm-chip { padding: 5px 12px; border-radius: 999px; background: alpha(currentColor, 0.07); }
window.pbpm .pbpm-chip label { font-size: 14px; }
window.pbpm .pbpm-dot { min-width: 18px; min-height: 18px; border-radius: 999px; }
window.pbpm .pbpm-dot.ready { background: #68b723; }
window.pbpm .pbpm-dot.check { background: #f9c440; }
window.pbpm .pbpm-dot.stop { background: #c6262e; }
window.pbpm label.pbpm-overall { font-size: 17px; font-weight: 700; }
window.pbpm label.pbpm-result { font-size: 22px; font-weight: 700; }
window.pbpm label.pbpm-today-total { font-size: 15px; font-weight: 600; }
window.pbpm button.pbpm-preset { padding: 6px 14px; }
window.pbpm .pbpm-sidebar-head { padding: 12px 14px 4px 14px; }
"""

STEPS = [("usb", "Printer connected"), ("queue", "Print queue set up"), ("ready", "Printer taking jobs"),
         ("file", "Label file valid"), ("sent", "Delivered to printer"), ("loaded", "Label confirmed")]
PRESETS = (1, 100, 500, 1000)
ICON = {"ok": "emblem-default", "bad": "dialog-error", "warn": "dialog-warning", "todo": "content-loading-symbolic"}


def gui():
    import gi
    gi.require_version("Gtk", "4.0")
    gi.require_version("Granite", "7.0")
    from gi.repository import Gtk, Gdk, GLib, Gio, Granite

    def lbl(text="", *classes, xalign=0.0, wrap=False):
        l = Gtk.Label(label=text, xalign=xalign, wrap=wrap)
        for c in classes: l.add_css_class(c)
        return l

    def margins(w, m):
        w.set_margin_top(m); w.set_margin_bottom(m); w.set_margin_start(m); w.set_margin_end(m)
        return w

    def icon_button(icon, tip, cb):
        b = Gtk.Button(icon_name=icon, tooltip_text=tip)
        b.update_property([Gtk.AccessibleProperty.LABEL], [tip])
        b.connect("clicked", cb)
        return b

    def card():
        b = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        b.add_css_class(Granite.STYLE_CLASS_CARD); b.add_css_class("pbpm-card")
        return b

    class Win(Gtk.ApplicationWindow):
        def __init__(self, app):
            super().__init__(application=app, default_width=1180, default_height=860, title="PolyBagger Print Manager")
            self.add_css_class("pbpm")
            self.busy = False; self.last = None; self.recent = {}; self.st = None
            self.last_target_id = None; self.last_connected = None
            self.usb_signature = tuple(d["identity"] for d in usb_printers())
            self.refresh_generation = 0; self._printer_ids = []; self._printer_options = []
            self._updating_printer_dropdown = False
            self.kinds = {}

            hb = Gtk.HeaderBar(); self.set_titlebar(hb)
            self.search = Gtk.SearchEntry(placeholder_text="Search labels", width_chars=34)
            self.search.connect("search-changed", lambda *_: self.on_search())
            hb.set_title_widget(self.search)
            hb.pack_start(icon_button("document-open-symbolic", "Open a ZPL file", self.on_browse))
            hb.pack_start(icon_button("view-refresh-symbolic", "Update the offline label store", self.on_sync))
            helpb = Gtk.MenuButton(icon_name="help-contents-symbolic", tooltip_text="How this machine prints")
            helpb.set_popover(self.build_help()); hb.pack_end(helpb)
            self.search.set_key_capture_widget(self)

            paned = Gtk.Paned(orientation=Gtk.Orientation.HORIZONTAL, position=380,
                              shrink_start_child=False, resize_start_child=False, vexpand=True)
            root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
            root.append(self.build_statusbar()); root.append(paned)
            self.set_child(root)

            # sidebar: searchable label library
            self.listbox = Gtk.ListBox(selection_mode=Gtk.SelectionMode.SINGLE)
            self.listbox.add_css_class("pbpm-sidebar")
            self.listbox.set_filter_func(self.row_visible)
            self.listbox.set_header_func(self.row_header)
            self.listbox.connect("row-selected", lambda *_: self.update_preview())
            self.listbox.set_placeholder(Granite.Placeholder(
                title="No labels", description="Nothing matches the search, or the label store is empty. "
                "Use the refresh button to sync from GitHub.", icon=Gio.ThemedIcon.new("edit-find-symbolic")))
            paned.set_start_child(Gtk.ScrolledWindow(child=self.listbox, hscrollbar_policy=Gtk.PolicyType.NEVER))

            # detail
            right = margins(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=18), 24)
            overlay = Gtk.Overlay(child=Gtk.ScrolledWindow(child=right, hscrollbar_policy=Gtk.PolicyType.NEVER))
            self.toast = Granite.Toast(title=""); overlay.add_overlay(self.toast)
            paned.set_end_child(overlay)

            banner = Gtk.Box(spacing=18)
            self.big = Gtk.Image(pixel_size=64)
            tb = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4, valign=Gtk.Align.CENTER, hexpand=True)
            self.title = lbl("", "pbpm-title"); self.sub = lbl("", "pbpm-sub", wrap=True)
            tb.append(self.title); tb.append(self.sub)
            banner.append(self.big); banner.append(tb); right.append(banner)

            # fixes appear here only when something needs one
            fixes = Gtk.Box(spacing=10)
            self.cancel_btn = Gtk.Button(label="Cancel Stuck Jobs"); self.cancel_btn.add_css_class("destructive-action")
            self.cancel_btn.connect("clicked", self.on_cancel_jobs)
            self.setup_btn = Gtk.Button(label="Set Up Print Queue…"); self.setup_btn.connect("clicked", self.on_setup)
            self.resume_btn = Gtk.Button(label="Resume Queue…"); self.resume_btn.add_css_class("suggested-action")
            self.resume_btn.set_tooltip_text("Resume the paused queue and stop it pausing after a printer power cycle (admin password)")
            self.resume_btn.connect("clicked", self.on_resume)
            for w in (self.resume_btn, self.cancel_btn, self.setup_btn): fixes.append(w)
            right.append(fixes)

            # the job: which label, how many, print
            jc = card()
            self.pname = lbl("Select a label", "pbpm-row-title")
            self.kind_lbl = lbl("", "pbpm-sub", wrap=True)
            self.preview = Gtk.Picture(can_shrink=True, content_fit=Gtk.ContentFit.CONTAIN, height_request=170)
            self.preview_cap = lbl("", "pbpm-sub", wrap=True)
            for w in (self.pname, self.kind_lbl, self.preview, self.preview_cap): jc.append(w)
            row = Gtk.Box(spacing=8, margin_top=6)
            self.qty_lbl = lbl("How many")
            self.qty = Gtk.SpinButton.new_with_range(1, 10000, 1); self.qty.set_value(load_settings().get("qty", 1))
            self.qty.set_tooltip_text("Label quantity sent as ^PQ; only use a printer that supports ZPL quantity")
            self.qty.update_property([Gtk.AccessibleProperty.LABEL], ["Labels to print"])
            self.qty.connect("value-changed", lambda w: (save_settings(qty=w.get_value_as_int()), self.update_send_label()))
            row.append(self.qty_lbl); row.append(self.qty)
            self.presets = []
            for n in PRESETS:
                b = Gtk.Button(label=str(n), tooltip_text=f"Print {n}"); b.add_css_class("pbpm-preset")
                b.connect("clicked", lambda _b, n=n: self.qty.set_value(n)); row.append(b); self.presets.append(b)
            row.append(Gtk.Box(hexpand=True))
            self.send_btn = Gtk.Button(label="Print"); self.send_btn.add_css_class("suggested-action")
            self.send_btn.add_css_class("pbpm-send"); self.send_btn.connect("clicked", self.on_send)
            self.send_btn.set_tooltip_text("Send to the printer (Ctrl+P)")
            row.append(self.send_btn); jc.append(row)
            right.append(jc)

            # result of the last job, with the one check that matters
            self.ask_card = card()
            rh = Gtk.Box(spacing=14)
            self.res_icon = Gtk.Image(pixel_size=40, valign=Gtk.Align.START)
            rt = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4, hexpand=True)
            self.res_title = lbl("", "pbpm-result", wrap=True); self.ask_text = lbl("", wrap=True)
            rt.append(self.res_title); rt.append(self.ask_text); rh.append(self.res_icon); rh.append(rt)
            self.ask_buttons = Gtk.Box(spacing=8, halign=Gtk.Align.END)
            self.ask_card.append(rh); self.ask_card.append(self.ask_buttons)
            self.ask = Gtk.Revealer(child=self.ask_card, reveal_child=False)
            right.append(self.ask)

            # today: what went to the printer
            tc = card()
            th = Gtk.Box(spacing=8); th.append(Granite.HeaderLabel(label="Today", hexpand=True))
            self.today_total = lbl("", "pbpm-today-total"); th.append(self.today_total); tc.append(th)
            self.today = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE); self.today.add_css_class("pbpm-today")
            self.today.set_placeholder(lbl("No jobs yet today.", "pbpm-sub", xalign=0.5))
            tc.append(self.today); right.append(tc)

            # details, collapsed: checks, test label, log
            details = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12, margin_top=8)
            grid = Gtk.Grid(row_spacing=10, column_spacing=12); self.rows = {}
            for i, (k, text) in enumerate(STEPS):
                img = Gtk.Image(icon_name=ICON["todo"], pixel_size=20); sp = Gtk.Spinner()
                st = Gtk.Stack(); st.add_named(img, "img"); st.add_named(sp, "spin")
                det = lbl("", "pbpm-sub", xalign=1.0); det.set_hexpand(True); det.set_ellipsize(3)
                grid.attach(st, 0, i, 1, 1); grid.attach(lbl(text), 1, i, 1, 1); grid.attach(det, 2, i, 1, 1)
                self.rows[k] = (img, sp, st, det)
            details.append(grid)
            self.test_btn = Gtk.Button(label="Send Test Label", halign=Gtk.Align.START); self.test_btn.connect("clicked", self.on_test)
            details.append(self.test_btn)
            self.log = Gtk.TextView(editable=False, monospace=True, wrap_mode=Gtk.WrapMode.WORD_CHAR)
            details.append(Gtk.ScrolledWindow(child=self.log, min_content_height=120))
            right.append(Gtk.Expander(label="Details: checks, test label, activity log", child=details))

            ctrl = Gtk.ShortcutController(); ctrl.set_scope(Gtk.ShortcutScope.GLOBAL)
            ctrl.add_shortcut(Gtk.Shortcut(trigger=Gtk.ShortcutTrigger.parse_string("<Control>p"),
                                           action=Gtk.CallbackAction.new(lambda *_: (self.send_btn.get_sensitive() and self.on_send(None), True)[1])))
            self.add_controller(ctrl)

            self.library = list(label_files())
            self.rebuild_sidebar()
            self.apply_status(status(), first=True)
            self.load_today(); self.update_send_label()
            GLib.timeout_add_seconds(1, self.poll)
            GLib.timeout_add_seconds(5, self.poll_queue)
            self.maybe_snapshot()

        # -- help popover: machine flow + known issues
        def build_help(self):
            box = margins(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8, width_request=460), 16)
            box.add_css_class("pbpm-help")
            box.append(Granite.HeaderLabel(label="Choosing a printer"))
            for t in ("Choose a configured CUPS printer from the Printer menu. Network printers must be added in System Settings first.",
                      "A new USB printer gets its own raw ZPL queue named from its serial. The printer selection and loaded-label status are remembered separately for each queue.",
                      "USB connection changes are detected automatically. Reconnect the same printer and its queue becomes available again."):
                box.append(lbl(t, wrap=True))
            box.append(Granite.HeaderLabel(label="How the T-1000 printer prints"))
            for i, t in enumerate(MACHINE_FLOW, 1): box.append(lbl(f"{i}. {t}", wrap=True))
            box.append(Granite.HeaderLabel(label="Known issues"))
            for t in KNOWN_ISSUES: box.append(lbl(f"• {t}", wrap=True))
            return Gtk.Popover(child=Gtk.ScrolledWindow(child=box, propagate_natural_height=True, max_content_height=640,
                                                        hscrollbar_policy=Gtk.PolicyType.NEVER))

        # -- status bar: what the computer sees right now
        def build_statusbar(self):
            bar = Gtk.Box(spacing=10); bar.add_css_class("pbpm-statusbar")
            overall = Gtk.Box(spacing=10, margin_end=8, valign=Gtk.Align.CENTER)
            self.dot = Gtk.Box(valign=Gtk.Align.CENTER); self.dot.add_css_class("pbpm-dot")
            self.overall = lbl("", "pbpm-overall")
            overall.append(self.dot); overall.append(self.overall); bar.append(overall)
            picker = Gtk.Box(spacing=6, valign=Gtk.Align.CENTER)
            picker.append(lbl("Printer"))
            self.printer_dropdown = Gtk.DropDown(model=Gtk.StringList.new(["Detecting printers…"]))
            self.printer_dropdown.set_size_request(250, -1)
            self.printer_dropdown.set_tooltip_text("Choose a configured ZPL printer or a USB printer to set up")
            self.printer_dropdown.update_property([Gtk.AccessibleProperty.LABEL], ["Printer"])
            self.printer_dropdown.connect("notify::selected", self.on_printer_changed)
            picker.append(self.printer_dropdown); bar.append(picker)
            self.chips = {}
            for key, tip in (("printer", "Selected printer connection or CUPS route"),
                             ("queue", "Print queue state"), ("label", "Label on the printer (last known)"),
                             ("job", "Last job")):
                chip = Gtk.Box(spacing=6, tooltip_text=tip); chip.add_css_class("pbpm-chip")
                img = Gtk.Image(pixel_size=16); text = lbl("")
                chip.append(img); chip.append(text); bar.append(chip)
                self.chips[key] = (img, text)
            return bar

        def update_overall(self):
            st = self.st or {}
            ls = self.current_label_state().get("state")
            if not st.get("ready"):
                state, text = "stop", "Not ready"
            elif ls == "in":
                state, text = "ready", "Ready"
            elif ls == "sent":
                state, text = "check", "Confirm label"
            else:
                state, text = "check", "Ready, no label"
            for c in ("ready", "check", "stop"): self.dot.remove_css_class(c)
            self.dot.add_css_class(state); self.overall.set_text(text)
            self.dot.update_property([Gtk.AccessibleProperty.LABEL], [text])

        def chip(self, key, icon, text):
            img, l = self.chips[key]; img.set_from_icon_name(ICON[icon]); l.set_text(text)

        def show_label_state(self):
            ls = self.current_label_state()
            name = Path(ls.get("file", "")).stem.replace("_", " ")
            at = f" · {ls['time'][11:16]}" if ls.get("time") else ""
            state = ls.get("state")
            if state == "in":
                self.chip("label", "ok", f"Label in: {name} × {ls.get('qty', 1)}{at}")
            elif state == "sent":
                self.chip("label", "warn", f"Sent, not confirmed: {name} × {ls.get('qty', 1)}{at}")
            elif state == "none":
                self.chip("label", "bad", f"No label{at}")
            else:
                self.chip("label", "todo", "Label: unknown")
            self.update_overall()

        def set_label_state(self, state, **kv):
            settings = load_settings()
            target_id = (self.st or {}).get("selected_target")
            if not target_id:
                return
            entry = {"state": state, "time": time.strftime("%Y-%m-%d %H:%M:%S"), **kv}
            states = dict(settings.get("label_states", {}))
            states[target_id] = entry
            save_settings(printer_target=target_id, label_states=states, label_state=entry)
            self.show_label_state()

        def current_label_state(self):
            settings = load_settings()
            target_id = (self.st or {}).get("selected_target")
            states = settings.get("label_states", {})
            if target_id and target_id in states:
                return states[target_id]
            if target_id and target_id == settings.get("printer_target"):
                return settings.get("label_state") or {}
            # Migrate the old single-printer state to the first selected target.
            if target_id and not states and not settings.get("printer_target") and settings.get("label_state"):
                legacy = settings["label_state"]
                save_settings(printer_target=target_id, label_states={target_id: legacy})
                return legacy
            return {}

        def update_printer_selector(self, st):
            targets = st.get("targets", [])
            options = [(t["id"], t["display"]) for t in targets] or [(None, "No printers found")]
            selected_id = st.get("selected_target")
            index = next((i for i, (identity, _) in enumerate(options) if identity == selected_id), 0)
            if options != self._printer_options:
                self._updating_printer_dropdown = True
                self._printer_options = options
                self._printer_ids = [identity for identity, _ in options]
                self.printer_dropdown.set_model(Gtk.StringList.new([label for _, label in options]))
                self.printer_dropdown.set_selected(index)
                self._updating_printer_dropdown = False
            elif self.printer_dropdown.get_selected() != index:
                self._updating_printer_dropdown = True
                self.printer_dropdown.set_selected(index)
                self._updating_printer_dropdown = False
            self.printer_dropdown.set_sensitive(bool(targets) and not self.busy)

        def on_printer_changed(self, *_):
            if self._updating_printer_dropdown:
                return
            index = self.printer_dropdown.get_selected()
            if index >= len(self._printer_ids):
                return
            target_id = self._printer_ids[index]
            if target_id and target_id != (self.st or {}).get("selected_target"):
                save_settings(printer_target=target_id)
                self.refresh()

        # -- label list: favorites, recents, then the whole library
        def rebuild_sidebar(self, select=None):
            select = select or self.current_file()
            child = self.listbox.get_first_child()
            while child:
                nxt = child.get_next_sibling()
                if isinstance(child, Gtk.ListBoxRow): self.listbox.remove(child)
                child = nxt
            st = load_settings()
            known = {str(p): p for p in self.library}
            favs = [known[f] for f in st.get("favorites", []) if f in known]
            recents = [known[f] for f in st.get("recents", []) if f in known][:6]
            target = None
            for section, paths in (("Favorites", favs), ("Recent", recents), ("All labels", self.library)):
                for p in paths:
                    row = self.add_row(p, section)
                    if target is None and select and p == select: target = row
            target = target or self.listbox.get_row_at_index(0)
            if target: self.listbox.select_row(target)

        def add_row(self, path, section="All labels"):
            self.kinds[path] = kind = kind_of(path)
            size = path.stat().st_size
            text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2, hexpand=True)
            text.append(lbl(path.stem.replace("_", " "), "pbpm-row-title"))
            text.append(lbl(f"{kind} · {size // 1024} KB" if size >= 1024 else f"{kind} · {size} bytes", "pbpm-sub"))
            fav = str(path) in load_settings().get("favorites", [])
            star = Gtk.Button(icon_name="starred-symbolic" if fav else "non-starred-symbolic",
                              tooltip_text="Remove from favorites" if fav else "Add to favorites", valign=Gtk.Align.CENTER)
            star.add_css_class("flat")
            star.update_property([Gtk.AccessibleProperty.LABEL], ["Remove from favorites" if fav else "Add to favorites"])
            star.connect("clicked", lambda _b, p=path: self.toggle_favorite(p))
            box = Gtk.Box(spacing=8); box.append(text); box.append(star)
            row = Gtk.ListBoxRow(child=box); row.path = path; row.section = section
            self.listbox.append(row)
            return row

        def toggle_favorite(self, path):
            favs = load_settings().get("favorites", [])
            favs = [f for f in favs if f != str(path)] if str(path) in favs else favs + [str(path)]
            save_settings(favorites=favs); self.rebuild_sidebar(select=path)

        def remember_recent(self, path):
            rec = [str(path)] + [r for r in load_settings().get("recents", []) if r != str(path)]
            save_settings(recents=rec[:6]); self.rebuild_sidebar(select=path)

        def row_header(self, row, before):
            # while searching only the full library is shown, so no section headers
            if self.search.get_text().strip():
                row.set_header(None); return
            if before is None or before.section != row.section:
                h = Granite.HeaderLabel(label=row.section); h.add_css_class("pbpm-sidebar-head")
                row.set_header(h)
            else:
                row.set_header(None)

        def row_visible(self, row):
            q = self.search.get_text().strip().lower()
            if not q: return True
            if row.section != "All labels": return False
            hay = (row.path.stem + " " + self.kinds.get(row.path, "")).lower().replace("_", " ")
            return all(w in hay for w in q.split())

        def on_search(self):
            self.listbox.invalidate_filter(); self.listbox.invalidate_headers()
            sel = self.listbox.get_selected_row()
            if sel is None or not sel.get_child_visible() or not self.row_visible(sel):
                i = 0
                while (r := self.listbox.get_row_at_index(i)) is not None:
                    if self.row_visible(r): self.listbox.select_row(r); break
                    i += 1

        def current_file(self):
            r = self.listbox.get_selected_row(); return r.path if r else None

        def say(self, msg):
            b = self.log.get_buffer(); b.insert(b.get_end_iter(), time.strftime("%H:%M:%S ") + msg + "\n")

        def notify(self, msg):
            self.say(msg); self.toast.set_title(msg); self.toast.send_notification()

        def mark(self, key, state, detail=""):
            img, sp, st, det = self.rows[key]
            if state == "work": sp.start(); st.set_visible_child_name("spin")
            else: sp.stop(); img.set_from_icon_name(ICON[state]); st.set_visible_child_name("img")
            det.set_text(detail); det.set_tooltip_text(detail or None)

        def update_preview(self):
            p = self.current_file()
            if not p: return
            self.pname.set_text(p.stem.replace("_", " "))
            self.kind_lbl.set_text(expectation(p)[1])
            self.update_send_label()
            try:
                img, cap = label_preview(p)
            except Exception as e:  # a preview must never block printing
                img, cap = None, f"No preview: {e}"
            if img:  # labels are sent upside down on purpose; show them inverted for auditing only
                w, h, rgb = img
                tex = Gdk.MemoryTexture.new(w, h, Gdk.MemoryFormat.R8G8B8, GLib.Bytes.new(rgb[::-1]), w * 3)
                self.preview.set_paintable(tex)
                cap += ". Preview inverted 180° for auditing only; the file is sent unchanged."
            else:
                self.preview.set_paintable(None)
            self.preview_cap.set_text(cap)

        # -- status
        def update_send_label(self):
            p = self.current_file()
            kind = expectation(p)[0] if p else "label"
            n = self.qty.get_value_as_int()
            store = kind == "store"
            for w in [self.qty, self.qty_lbl] + self.presets: w.set_sensitive(not store)
            self.send_btn.set_label("Save to Printer Flash" if store else f"Print {n:,} Label{'s' if n != 1 else ''}")

        def load_today(self):
            child = self.today.get_first_child()
            while child: nxt = child.get_next_sibling(); self.today.remove(child); child = nxt
            day = time.strftime("%Y-%m-%d"); jobs = []
            try:
                for line in LOG_PATH.read_text().splitlines():
                    e = json.loads(line)
                    if e.get("time", "").startswith(day) and "stage" in e: jobs.append(e)
            except (OSError, ValueError):
                pass
            ok = [j for j in jobs if j.get("ok")]
            labels = sum(int(j.get("quantity") or 1) for j in ok if expectation_kind(j.get("file", "")) != "store")
            self.today_total.set_text(f"{len(ok)} sent · {labels:,} labels" + (f" · {len(jobs) - len(ok)} failed" if len(jobs) > len(ok) else ""))
            for j in reversed(jobs[-10:]):
                r = Gtk.Box(spacing=12, margin_top=6, margin_bottom=6)
                r.append(Gtk.Image(icon_name=ICON["ok" if j.get("ok") else "bad"], pixel_size=18))
                r.append(lbl(j["time"][11:16], "pbpm-sub"))
                name = Path(j.get("file", "")).stem.replace("_", " ")
                if Path(j.get("file", "")).name.startswith("tmp"): name = "Test label"  # mkstemp test labels
                nl = lbl(name); nl.set_hexpand(True); r.append(nl)
                r.append(lbl(f"× {int(j.get('quantity') or 1):,}" if j.get("ok") else j.get("stage", "failed"), "pbpm-sub"))
                self.today.append(r)

        def poll(self):
            signature = tuple(d["identity"] for d in usb_printers())
            if signature != self.usb_signature:
                self.usb_signature = signature
                if not self.busy: self.refresh()
            return True

        def poll_queue(self):
            if not self.busy: self.refresh()
            return True

        def refresh(self):
            self.refresh_generation += 1
            generation = self.refresh_generation
            def work():
                st = status(); GLib.idle_add(self.apply_status, st, False, generation)
            threading.Thread(target=work, daemon=True).start()

        def apply_status(self, st, first=False, generation=None):
            if generation is not None and generation != self.refresh_generation:
                return False
            previous_target, previous_connected = self.last_target_id, self.last_connected
            self.st = st
            self.update_printer_selector(st)
            target = st.get("target")
            target_id = st.get("selected_target")
            connected = bool(st.get("connected"))
            if not first and target_id == previous_target and previous_connected is not None and connected != previous_connected:
                if connected:
                    self.notify("Printer reconnected")
                else:
                    self.notify("Printer unplugged")
                    if target and target.get("transport") == "usb":
                        self.set_label_state("unknown")  # a power cut or unplug drops the loaded label
            self.last_target_id, self.last_connected = target_id, connected
            stuck = "waiting-for-printer" in st["reasons"]
            d = st.get("device")
            if connected and d:
                ptext = f"Connected · {d['serial'] or d['product']}"
            elif connected and target:
                ptext = f"CUPS · {target['queue']}"
            elif target:
                ptext = f"Disconnected · {d['serial'] if d else target['queue']}"
            else:
                ptext = "No printer selected"
            self.chip("printer", "ok" if connected else "bad", ptext)
            q = ("ok", "Queue ready") if st["ready"] else \
                ("bad", "Queue needed") if target and target.get("needs_setup") else \
                ("bad", "Printer unplugged") if target and not connected else \
                ("bad", "Stalled: waiting for printer") if stuck else \
                ("bad", "Queue paused") if st["queue_state"] == "disabled" else \
                ("bad", "No printer selected") if not target else ("warn", f"Queue {st['queue_state']}")
            self.chip("queue", *q)
            if first: self.chip("job", "todo", "No job yet this session")
            self.show_label_state()
            pdetail = f"{d['product']} · {d['serial'] or d['bus']}" if d else (f"CUPS · {st['queue']}" if st["queue"] else "not selected")
            self.mark("usb", "ok" if connected else "bad", pdetail)
            self.mark("queue", "ok" if st["queue"] else ("bad" if target and target.get("needs_setup") else "todo"),
                      st["queue"] or ("set up this USB printer" if target else "none"))
            self.mark("ready", "ok" if st["ready"] else ("bad" if target else "todo"),
                      "stuck: waiting for printer" if stuck else st["queue_state"] + (f" · {' '.join(st['reasons'])}" if st["reasons"] else ""))
            if self.busy: return False
            if st["ready"]:
                icon, title, sub = "printer", "Printer ready", "Pick a label and send it. Confirm the label at the printer after delivery."
            elif not target:
                icon, title, sub = "printer-error", "No printers found", "Connect a USB printer or add a printer in System Settings."
            elif target.get("needs_setup"):
                icon, title, sub = "printer-error", "Print queue needed", "Set up a raw ZPL queue for this USB printer."
            elif not connected:
                icon, title, sub = "printer-error", "Printer unplugged", "Reconnect this printer. The app will detect it and restore its queue automatically."
            elif stuck:
                icon, title, sub = "printer-error", "Printer not taking jobs", STUCK_HINT
            elif st["queue_state"] == "disabled":
                icon, title, sub = "printer-error", "Print queue paused", ("The printer was switched off or lost during a job. "
                                                                           "Press Resume Queue (admin password).")
            else:
                icon, title, sub = "printer-error", "Printer not ready", "Check the printer alerts in the checks below."
            self.big.set_from_icon_name(icon); self.title.set_text(title); self.sub.set_text(sub)
            self.setup_btn.set_visible(bool(target and target.get("needs_setup")))
            self.cancel_btn.set_visible(stuck)
            self.resume_btn.set_visible(bool(st["queue"]) and st["queue_state"] == "disabled")
            self.send_btn.set_sensitive(st["ready"]); self.test_btn.set_sensitive(st["ready"])
            self.printer_dropdown.set_sensitive(bool(st.get("targets")) and not self.busy)
            return False

        # -- actions
        def on_browse(self, _):
            dlg = Gtk.FileDialog(title="Open ZPL file")
            f = Gtk.FileFilter(name="ZPL labels"); f.add_pattern("*.zpl"); f.add_pattern("*.ZPL")
            filters = Gio.ListStore.new(Gtk.FileFilter); filters.append(f); dlg.set_filters(filters)
            def done(d, res):
                try: gf = d.open_finish(res)
                except GLib.Error: return
                p = Path(gf.get_path())
                if p not in self.library: self.library.append(p)
                self.rebuild_sidebar(select=p)
            dlg.open(self, None, done)

        def on_sync(self, _):
            self.say("Updating the label store…")
            def work():
                c, n, note = sync_library(pull=True)
                def done():
                    self.library = list(label_files()) + [p for p in self.library if p.parent != STORE]
                    self.rebuild_sidebar()
                    self.notify(f"Label store: {n} labels, {c} updated")
                    if note: self.say(note)
                GLib.idle_add(done)
            threading.Thread(target=work, daemon=True).start()

        def on_setup(self, _):
            target = (self.st or {}).get("target")
            dev = target.get("device") if target else None
            if not dev or not target.get("needs_setup"):
                return
            self.say("Creating the print queue (admin password required)…")
            def work():
                rc, out, queue = setup_queue(dev)
                def done():
                    if rc == 0:
                        save_settings(printer_target=queue)
                        self.say(f"Queue {queue} is ready")
                    else:
                        self.say(f"Setup failed: {out}")
                    self.refresh()
                GLib.idle_add(done)
            threading.Thread(target=work, daemon=True).start()

        def on_resume(self, _):
            self.say("Resuming the print queue (admin password required)…")
            q = self.st["queue"]
            def work():
                done, _ = cancel_jobs(q)  # jobs left over from the power cut would otherwise print late
                rc, out = resume_queue(q)
                GLib.idle_add(lambda: (self.notify("Print queue resumed" if rc == 0 else "Queue not resumed"),
                                       self.say(out) if rc and out else None, self.refresh()))
            threading.Thread(target=work, daemon=True).start()

        def on_cancel_jobs(self, _):
            done, failed = cancel_jobs(self.st["queue"])
            self.notify(f"Cancelled {len(done)} stuck job(s)" + (f", {len(failed)} could not be cancelled" if failed else ""))
            self.refresh()

        def on_test(self, _):
            fd, tmp = tempfile.mkstemp(suffix=".zpl")
            os.write(fd, TEST_LABEL.format(stamp=time.strftime("%Y-%m-%d %H:%M:%S")).encode()); os.close(fd)
            self.start_send(Path(tmp), temp=True)

        def on_send(self, _):
            p = self.current_file()
            if not p or not self.st or not self.st.get("ready"): return
            last = self.recent.get(str(p))
            if last and time.time() - last < 120:
                dlg = Granite.MessageDialog.new_with_image_from_icon_name(
                    "Send this label again?",
                    f"{p.name} was sent {int(time.time() - last)} s ago. Each send loads the label again. If the bagger "
                    "shows ReprintBuffer or Reprint ON, it keeps printing on every cycle.",
                    "dialog-warning", Gtk.ButtonsType.CANCEL)
                dlg.set_transient_for(self); dlg.set_modal(True)
                dlg.add_button("Send Again", Gtk.ResponseType.ACCEPT).add_css_class("suggested-action")
                def resp(d, r):
                    d.destroy()
                    if r == Gtk.ResponseType.ACCEPT: self.start_send(p)
                dlg.connect("response", resp); dlg.present()
                return
            self.start_send(p)

        def start_send(self, path, temp=False):
            if not self.st or not self.st.get("ready") or not self.st.get("queue"):
                return
            for k in ("file", "sent", "loaded"): self.mark(k, "todo")
            ok, warns = check_zpl(path)
            if not ok: self.mark("file", "bad", warns[0]); return
            if not temp: self.recent[str(path)] = time.time()
            self.mark("file", "warn" if warns else "ok", f"{path.stat().st_size // 1024} KB" + (" · see log" if warns else ""))
            for w in warns: self.say("WARNING: " + w)
            self.busy = True; self.send_btn.set_sensitive(False); self.test_btn.set_sensitive(False)
            self.printer_dropdown.set_sensitive(False)
            self.ask.set_reveal_child(False); self.mark("sent", "work")
            self.title.set_text("Sending…"); self.sub.set_text(path.name)
            queue = self.st["queue"]
            target_id = self.st["selected_target"]
            qty = 1 if temp else self.qty.get_value_as_int()
            def work():
                job_path, job_temp = with_quantity(path, qty)
                try:
                    res = send_and_verify(job_path, queue, progress=lambda m: GLib.idle_add(self.say, m))
                finally:
                    if job_temp: os.unlink(job_path)
                res.update(quantity=qty, queue=queue, printer_target=target_id)
                GLib.idle_add(self.finish, path, res, temp)
            threading.Thread(target=work, daemon=True).start()

        def finish(self, path, res, temp):
            self.busy = False; res["file"] = str(path)
            kind, hint = expectation(path); res["expect"] = hint
            log_result(res); self.last = res
            self.say(("DELIVERED " if res["ok"] else "FAILED ") + res["detail"])
            self.chip("job", "ok" if res["ok"] else "bad",
                      f"{'Delivered' if res['ok'] else 'Not delivered'} · {path.name if not temp else 'test label'} · {time.strftime('%H:%M')}")
            if res["ok"] and not temp: self.remember_recent(path)
            if res["ok"] and kind != "store": self.set_label_state("sent", file=str(path), qty=res.get("quantity", 1))
            name = "Test label" if temp else path.stem.replace("_", " ")
            if res["ok"]:
                self.mark("sent", "ok", f"{res['job']} · {res['seconds']} s")
                self.big.set_from_icon_name("printer")
                if kind == "store":
                    self.show_result("ok", f"Saved {name} to printer flash",
                                     "Nothing loads on the bagger from a store file. Print the recall file next.", [])
                else:
                    self.title.set_text("Printer received the label data")
                    self.sub.set_text("Confirm the label is ready at this printer.")
                    self.show_result("ok", f"Sent {name} × {res['quantity']:,}",
                                     hint + " Did it load or print the label?",
                                     [("Not Ready Yet", "no"), ("Label Is Ready", "yes")])
            else:
                self.mark("sent", "bad", res["stage"])
                self.big.set_from_icon_name("printer-error")
                self.title.set_text("Label not delivered"); self.sub.set_text("See the result below.")
                self.show_result("bad", f"{name} did not reach the printer", res["detail"], [("Try Again", "retry")])
            self.load_today()
            if temp: os.unlink(path)
            self.refresh()
            return False

        # -- result card: what happened, and the one check on the bagger
        def show_result(self, state, title, text, opts):
            self.res_icon.set_from_icon_name(ICON[state]); self.res_title.set_text(title); self.ask_text.set_text(text)
            child = self.ask_buttons.get_first_child()
            while child: nxt = child.get_next_sibling(); self.ask_buttons.remove(child); child = nxt
            for label, val in opts:
                b = Gtk.Button(label=label)
                if val in ("yes", "retry"): b.add_css_class("suggested-action")
                b.connect("clicked", lambda _b, v=val: self.on_answer(v))
                self.ask_buttons.append(b)
            self.ask_buttons.set_visible(bool(opts))
            self.ask.set_reveal_child(True)

        def on_answer(self, val):
            if val == "retry":
                self.on_send(None); return
            entry = {"job": self.last.get("job"), "file": self.last.get("file")}
            loaded = val == "yes"
            log_result({**entry, "printer_confirmed": loaded})
            name = Path(entry["file"] or "").stem.replace("_", " ")
            qty = self.last.get("quantity", 1)
            if loaded:
                self.set_label_state("in", file=entry["file"], qty=qty)
                self.mark("loaded", "ok", "operator confirmed")
                self.show_result("ok", f"Ready: {name} × {qty:,} at the printer",
                                 "The label is confirmed. Continue with the printer's normal run workflow.", [])
            else:
                self.set_label_state("none"); self.mark("loaded", "bad", "still No Label")
                self.show_result("bad", "The label is not ready at the printer",
                                 "Check that the printer is powered on and online. On a TI-1000Z, check Printer ON and Run mode, then try again.",
                                 [("Try Again", "retry")])

        # -- dev aid: PBPM_SHOT=/path.png renders the window to a PNG, then quits
        def maybe_snapshot(self):
            shot = os.environ.get("PBPM_SHOT")
            if not shot: return
            if os.environ.get("PBPM_SHOT_H"): self.set_default_size(1180, int(os.environ["PBPM_SHOT_H"]))
            if os.environ.get("PBPM_SHOT_ASK"):
                self.show_result("ok", "Sent CCSBLACK recall flash × 1,000", expectation(self.current_file())[1] + " Does it?",
                                 [("Still No Label", "no"), ("Yes, Label Is In", "yes")])
            def snap():
                w, h = self.get_width(), self.get_height()
                paint = Gtk.WidgetPaintable.new(self)
                snapshot = Gtk.Snapshot(); paint.snapshot(snapshot, w, h)
                node = snapshot.to_node()
                if node: self.get_native().get_renderer().render_texture(node, None).save_to_png(shot)
                self.get_application().quit(); return False
            GLib.timeout_add(1800, snap)

    class App(Gtk.Application):
        def __init__(self):
            super().__init__(application_id="io.techmore.PolyBaggerPrintManager", flags=Gio.ApplicationFlags.DEFAULT_FLAGS)

        def do_startup(self):
            Gtk.Application.do_startup(self)
            Granite.init()
            # the icon ships next to this file; install.sh also puts it in hicolor for the dock and app grid
            Gtk.IconTheme.get_for_display(Gdk.Display.get_default()).add_search_path(str(Path(__file__).resolve().parent))
            Gtk.Window.set_default_icon_name("io.techmore.PolyBaggerPrintManager")
            gs, gtk = Granite.Settings.get_default(), Gtk.Settings.get_default()
            dark = lambda *_: setattr(gtk.props, "gtk_application_prefer_dark_theme",
                                      gs.props.prefers_color_scheme == Granite.SettingsColorScheme.DARK)
            dark(); gs.connect("notify::prefers-color-scheme", dark)
            prov = Gtk.CssProvider(); prov.load_from_string(CSS)
            Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), prov, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

        def do_activate(self):
            (self.props.active_window or Win(self)).present()

    return App().run([])


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--status", action="store_true"); ap.add_argument("--send", metavar="FILE")
    ap.add_argument("--cancel", action="store_true", help="cancel your stuck jobs on the queue")
    ap.add_argument("--qty", type=int, default=1, metavar="N", help="with --send: labels to print (^PQ), default 1")
    ap.add_argument("--watch", action="store_true"); ap.add_argument("--setup", action="store_true")
    ap.add_argument("--sync", action="store_true", help="refresh the offline label store"); ap.add_argument("--pull", action="store_true", help="with --sync: git pull the thril repo first")
    ap.add_argument("--printer", help="select a CUPS queue, USB serial, or USB identity")
    ap.add_argument("--json", action="store_true"); ap.add_argument("--yes", action="store_true", help="skip the 'did it print?' prompt")
    a = ap.parse_args()
    sys.exit(cli(a) if (a.status or a.send or a.watch or a.setup or a.sync or a.cancel) else gui())
