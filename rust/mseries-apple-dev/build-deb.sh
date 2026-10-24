#!/usr/bin/env bash
# Build the mseries-apple-dev Debian package (release build).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"
out="${MSERIES_APPLE_DEV_OUTPUT_DIR:-$repo_root/build/mseries-apple-dev-deb}"
id=org.techmore.mseries.AppleDev

for tool in cargo dpkg-deb; do
  command -v "$tool" >/dev/null || { echo "Missing build tool: $tool" >&2; exit 2; }
done

version="$(grep -m1 -oP '^version = "\K[0-9.]+' "$here/Cargo.toml")"
arch="$(dpkg --print-architecture)"
debver="${version}-1~mseries1"
mkdir -p "$out"
stage="$(mktemp -d "$out/package.XXXXXX")"
trap 'rm -rf "$stage"' EXIT

(cd "$repo_root/rust" && cargo build --release --locked -p mseries-apple-dev)
target_dir="$(cd "$repo_root/rust" && cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
install -Dm755 "$target_dir/release/mseries-apple-dev" "$stage/usr/bin/mseries-apple-dev"
install -Dm644 "$here/data/$id.desktop" "$stage/usr/share/applications/$id.desktop"
install -Dm644 "$here/data/$id.metainfo.xml" "$stage/usr/share/metainfo/$id.metainfo.xml"
install -Dm644 "$here/data/$id.svg" "$stage/usr/share/icons/hicolor/scalable/apps/$id.svg"
install -Dm644 "$here/data/$id-symbolic.svg" "$stage/usr/share/icons/hicolor/symbolic/apps/$id-symbolic.svg"

find "$stage" -type d -exec chmod 0755 {} +
find "$stage/usr/share" -type f -exec chmod 0644 {} +
installed_kb="$(du -sk "$stage" | cut -f1)"
install -d "$stage/DEBIAN"
cat >"$stage/DEBIAN/control" <<CONTROL
Package: mseries-apple-dev
Version: $debver
Section: devel
Priority: optional
Architecture: $arch
Installed-Size: $installed_kb
Depends: libgtk-4-1 (>= 4.20), libgranite-7-7 (>= 7.8) | libgranite7 (>= 7.8), libglib2.0-0, libvte-2.91-gtk4-0 | libvte-2.91-gtk4-0t64, git
Recommends: usbmuxd
Suggests: libimobiledevice-utils
Maintainer: elementaryos-mseries local build
Homepage: https://github.com/techmore/ElementaryOS_MSeries
Description: Apple Dev: build, run and ship iPhone apps from Linux
 Native GTK 4 and Granite front end for omarchy-apple-dev: toolchain status
 and install, connected iPhones, and per-project build, run, LLDB, App Store
 and TestFlight actions, run in an embedded VTE terminal.
CONTROL

pkg="$out/mseries-apple-dev_${debver}_${arch}.deb"
dpkg-deb --root-owner-group --build "$stage" "$pkg" >/dev/null
echo "Built $pkg"
dpkg-deb --info "$pkg" | sed -n '1,12p'
dpkg-deb --contents "$pkg" | awk '{print $1, $3, $6}'
