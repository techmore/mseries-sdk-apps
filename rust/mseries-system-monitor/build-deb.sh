#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"
out="${MSERIES_SYSTEM_MONITOR_OUTPUT_DIR:-$repo_root/build/mseries-system-monitor-deb}"
id=org.techmore.mseries.SystemMonitor

for tool in cargo dpkg-deb; do
  command -v "$tool" >/dev/null || { echo "Missing build tool: $tool" >&2; exit 2; }
done

version="$(grep -m1 -oP '^version = "\K[0-9.]+' "$here/Cargo.toml")"
arch="$(dpkg --print-architecture)"
debver="${version}-1~mseries1"
mkdir -p "$out"
stage="$(mktemp -d "$out/package.XXXXXX")"
trap 'rm -rf "$stage"' EXIT

(cd "$repo_root/rust" && cargo build --release --locked -p mseries-system-monitor)
target_dir="$(cd "$repo_root/rust" && cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
install -Dm755 "$target_dir/release/mseries-system-monitor" "$stage/usr/bin/mseries-system-monitor"
install -Dm644 "$here/data/$id.desktop" "$stage/usr/share/applications/$id.desktop"
install -Dm644 "$here/data/$id.metainfo.xml" "$stage/usr/share/metainfo/$id.metainfo.xml"

find "$stage" -type d -exec chmod 0755 {} +
find "$stage/usr/share" -type f -exec chmod 0644 {} +
installed_kb="$(du -sk "$stage" | cut -f1)"
install -d "$stage/DEBIAN"
cat >"$stage/DEBIAN/control" <<CONTROL
Package: mseries-system-monitor
Version: $debver
Section: admin
Priority: optional
Architecture: $arch
Installed-Size: $installed_kb
Depends: libgtk-4-1 (>= 4.20), libgranite-7-7 (>= 7.8) | libgranite7 (>= 7.8), libglib2.0-0, libvte-2.91-gtk4-0 | libvte-2.91-gtk4-0t64
Suggests: btop
Maintainer: elementaryos-mseries local build
Homepage: https://github.com/techmore/ElementaryOS_MSeries
Description: Pantheon System Monitor with btop and host resource metrics
 Native GTK 4 and Granite host CPU and memory overview with btop process
 detail in an embedded VTE terminal.
CONTROL

pkg="$out/mseries-system-monitor_${debver}_${arch}.deb"
dpkg-deb --root-owner-group --build "$stage" "$pkg" >/dev/null
echo "Built $pkg"
dpkg-deb --info "$pkg" | sed -n '1,12p'
dpkg-deb --contents "$pkg" | awk '{print $1, $3, $6}'
