#!/usr/bin/env bash
# Install the built preview for this user without replacing qualified Messages.
set -euo pipefail
if [[ $# != 2 || $1 != --deb ]]; then
  echo "Usage: $0 --deb /path/to/mseries-messages-rs.deb" >&2
  exit 2
fi
pkg=$(realpath -- "$2")
[[ $(dpkg-deb -f "$pkg" Package) == mseries-messages-rs ]] || {
  echo 'Expected a mseries-messages-rs package' >&2; exit 2;
}
[[ $(dpkg-deb -f "$pkg" Architecture) == "$(dpkg --print-architecture)" ]] || {
  echo 'Package architecture does not match this host' >&2; exit 2;
}
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
dpkg-deb -x "$pkg" "$stage"
bin="$HOME/.local/bin/mseries-messages-rs"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
install -Dm755 "$stage/usr/bin/mseries-messages-rs" "$bin"
install -Dm644 "$stage/usr/share/applications/io.elementary.mseries.MessagesRust.desktop" \
  "$data/applications/io.elementary.mseries.MessagesRust.desktop"
# Launch reliably even when the desktop session omits ~/.local/bin from PATH.
python3 - "$data/applications/io.elementary.mseries.MessagesRust.desktop" "$bin" <<'DESKTOP'
from pathlib import Path
import sys
p = Path(sys.argv[1])
command = sys.argv[2].replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%')
p.write_text('\n'.join('Exec="' + command + '"' if line.startswith('Exec=') else line
                        for line in p.read_text().splitlines()) + '\n')
DESKTOP
install -Dm644 "$stage/usr/share/icons/hicolor/scalable/apps/io.elementary.mseries.Messages.svg" \
  "$data/icons/hicolor/scalable/apps/io.elementary.mseries.Messages.svg"
install -Dm644 "$stage/usr/share/metainfo/io.elementary.mseries.MessagesRust.metainfo.xml" \
  "$data/metainfo/io.elementary.mseries.MessagesRust.metainfo.xml"
if command -v update-desktop-database >/dev/null; then
  update-desktop-database "$data/applications"
fi
printf 'Installed Messages (Rust Preview): %s\n' "$bin"
