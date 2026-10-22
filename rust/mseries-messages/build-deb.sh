#!/usr/bin/env bash
# Build an installable preview without replacing the current Messages launcher.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"
out="${MSERIES_MESSAGES_OUTPUT_DIR:-$repo_root/build/mseries-messages-rs-deb}"
id=io.elementary.mseries.MessagesRust

for tool in cargo dpkg-deb; do
  command -v "$tool" >/dev/null || { echo "Missing build tool: $tool" >&2; exit 2; }
done

version="$(grep -m1 -oP '^version = "\K[0-9.]+' "$here/Cargo.toml")"
arch="$(dpkg --print-architecture)"
debver="${version}-1~mseries1"
mkdir -p "$out"
stage="$(mktemp -d "$out/package.XXXXXX")"
trap 'rm -rf "$stage"' EXIT

(cd "$repo_root/rust" && cargo build --release --locked -p mseries-messages)
target_dir="$(cd "$repo_root/rust" && cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
install -Dm755 "$target_dir/release/mseries-messages" "$stage/usr/bin/mseries-messages-rs"
install -Dm644 "$here/data/$id.desktop" "$stage/usr/share/applications/$id.desktop"
install -Dm644 "$here/data/$id.metainfo.xml" "$stage/usr/share/metainfo/$id.metainfo.xml"
install -Dm644 "$repo_root/mseries/phone-setup/io.elementary.mseries.Messages.svg" \
  "$stage/usr/share/icons/hicolor/scalable/apps/io.elementary.mseries.Messages.svg"

find "$stage" -type d -exec chmod 0755 {} +
find "$stage/usr/share" -type f -exec chmod 0644 {} +
installed_kb="$(du -sk "$stage" | cut -f1)"
install -d "$stage/DEBIAN"
cat >"$stage/DEBIAN/control" <<CONTROL
Package: mseries-messages-rs
Version: $debver
Section: net
Priority: optional
Architecture: $arch
Installed-Size: $installed_kb
Depends: libgtk-4-1 (>= 4.20), libgranite-7-7 (>= 7.8) | libgranite7 (>= 7.8), libglib2.0-0, libsecret-tools
Suggests: blueferry
Maintainer: elementaryos-mseries local build
Homepage: https://github.com/techmore/ElementaryOS_MSeries
Description: Native Rust Pantheon Messages preview
 GTK 4 and Granite conversation client for the BlueFerry Messages API with
 phone-scoped imported Apple conversation pins in the desktop keyring.
 This preview installs alongside the current Messages app and does not replace
 its launcher.
CONTROL

pkg="$out/mseries-messages-rs_${debver}_${arch}.deb"
dpkg-deb --root-owner-group --build "$stage" "$pkg" >/dev/null
echo "Built $pkg"
dpkg-deb --info "$pkg" | sed -n '1,12p'
dpkg-deb --contents "$pkg" | awk '{print $1, $3, $6}'
