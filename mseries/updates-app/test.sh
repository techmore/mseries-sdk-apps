#!/usr/bin/env bash
# Tests for the Software Updates app: unit tests, the headless render/interaction test under Xvfb,
# and a staged install checked for the helper path, policy and launcher. Touches nothing outside
# CARGO_TARGET_DIR and a temporary directory; needs GTK 4, Granite 7, xvfb, dbus-run-session.
#
#   bash mseries/updates-app/test.sh                       # CARGO_TARGET_DIR defaults to a temp dir
#   MSERIES_UPDATES_SHOTS=/tmp/shots bash mseries/updates-app/test.sh   # keep screenshots (light + dark)
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/mseries-updates-app-test.XXXXXX")
trap 'rm -rf "$work"' EXIT
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$work/target}
manifest=$repo/rust/Cargo.toml

echo "== unit and helper tests"
cargo test --locked --manifest-path "$manifest" -p mseries-update-core -p mseries-updates --lib --bins --test helper

echo "== the real mseries-update status --json against the Rust parser"
cargo test --locked --manifest-path "$manifest" -p mseries-updates --test contract

echo "== headless render and interaction test (Xvfb)"
dbus-run-session -- env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GSETTINGS_BACKEND=memory GSK_RENDERER=cairo \
  xvfb-run -a -s "-screen 0 1280x1024x24" \
  cargo test --locked --manifest-path "$manifest" -p mseries-updates --test render

echo "== staged install"
helper=/usr/libexec/mseries-updates/mseries-update-helper
MSERIES_UPDATE_HELPER=$helper cargo build --locked --release --manifest-path "$manifest" -p mseries-updates -p mseries-update-core
DESTDIR=$work/root PREFIX=/usr bash "$repo/mseries/updates-app/install.sh" --no-build >/dev/null
policy=$work/root/usr/share/polkit-1/actions/org.techmore.mseries.update.policy
python3 - "$policy" "$helper" <<'PY'
import sys, xml.etree.ElementTree as ET
root = ET.parse(sys.argv[1]).getroot()
actions = root.findall("action")
assert [a.get("id") for a in actions] == ["org.techmore.mseries.update"], "exactly one action expected"
notes = {n.get("key"): n.text for n in actions[0].findall("annotate")}
assert notes["org.freedesktop.policykit.exec.path"] == sys.argv[2], notes
assert notes["org.freedesktop.policykit.exec.allow_gui"] == "true", notes
defaults = {d.tag: d.text for d in actions[0].find("defaults")}
assert defaults == {"allow_any": "auth_admin", "allow_inactive": "auth_admin", "allow_active": "auth_admin_keep"}, defaults
print("policy: one action, exec.path is the helper, admin authentication required")
PY
grep -q "$helper" "$work/root/usr/bin/mseries-updates" || { echo "FAIL: the app does not carry the helper path" >&2; exit 1; }
if ldd "$work/root/usr/libexec/mseries-updates/mseries-update-helper" | grep -qiE 'gtk|granite|glib'; then
  echo "FAIL: the root helper links a GUI toolkit" >&2
  exit 1
fi
if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$work/root/usr/share/applications/org.techmore.mseries.Updates.desktop"
fi
DESTDIR=$work/root PREFIX=/usr bash "$repo/mseries/updates-app/install.sh" --uninstall >/dev/null
[[ -z $(find "$work/root" -type f) ]] || { echo "FAIL: uninstall left files behind" >&2; find "$work/root" -type f >&2; exit 1; }
echo "install: staged, helper path in the app and the policy agree, helper is toolkit-free, uninstall is clean"
echo "PASS: Software Updates app tests"
