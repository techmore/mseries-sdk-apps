#!/usr/bin/env bash
# Tests for the Disk Space app: unit tests (scanner, layout, deletion guards), a scan checked against
# `du`, the headless render/interaction test under Xvfb, and a staged install. Touches nothing outside
# CARGO_TARGET_DIR and temporary directories; needs GTK 4, Granite 7, xvfb, dbus-run-session.
#
#   bash mseries/diskspace-app/test.sh                       # CARGO_TARGET_DIR defaults to a temp dir
#   MSERIES_DISKSPACE_SHOTS=/tmp/shots bash mseries/diskspace-app/test.sh   # keep screenshots (light + dark)
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/mseries-diskspace-test.XXXXXX")
trap 'rm -rf "$work"' EXIT
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$work/target}
manifest=$repo/rust/Cargo.toml

echo "== unit tests"
cargo test --locked --manifest-path "$manifest" -p mseries-diskspace --lib --bins

echo "== a real scan agrees with du (allocated bytes)"
cargo build --locked --manifest-path "$manifest" -p mseries-diskspace
for dir in "$repo/mseries" /usr/share/doc; do
  [[ -d $dir ]] || continue
  ours=$("$CARGO_TARGET_DIR/debug/mseries-diskspace" --scan "$dir" | head -1 | sed -E 's/^[^:]*: ([0-9]+) bytes on disk.*/\1/')
  theirs=$(du -sx --block-size=1 "$dir" 2>/dev/null | cut -f1)
  [[ $ours == "$theirs" ]] || { echo "FAIL: $dir: scan says $ours, du says $theirs" >&2; exit 1; }
  echo "ok: $dir = $ours bytes"
done

echo "== headless render and interaction test (Xvfb)"
dbus-run-session -- env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GSETTINGS_BACKEND=memory GSK_RENDERER=cairo \
  xvfb-run -a -s "-screen 0 1400x900x24" \
  cargo test --locked --manifest-path "$manifest" -p mseries-diskspace --test render

echo "== staged install"
cargo build --locked --release --manifest-path "$manifest" -p mseries-diskspace
DESTDIR=$work/root PREFIX=/usr bash "$repo/mseries/diskspace-app/install.sh" --no-build >/dev/null
[[ -x $work/root/usr/bin/mseries-diskspace ]] || { echo "FAIL: binary not staged" >&2; exit 1; }
grep -q '^Exec=mseries-diskspace$' "$work/root/usr/share/applications/org.techmore.mseries.DiskSpace.desktop"
if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$work/root/usr/share/applications/org.techmore.mseries.DiskSpace.desktop"
fi
DESTDIR=$work/root PREFIX=/usr bash "$repo/mseries/diskspace-app/install.sh" --uninstall >/dev/null
[[ ! -e $work/root/usr/bin/mseries-diskspace ]] || { echo "FAIL: uninstall left the binary" >&2; exit 1; }
echo "PASS: Disk Space app"
