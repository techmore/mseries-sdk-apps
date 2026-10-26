#!/usr/bin/env bash
# Build claude-pantheon-rs_<version>_<arch>.deb from this source tree: the app (binary, desktop entry with
# actions, D-Bus service, metainfo, hicolor icons) and the Wingpanel indicator.
#   bash rust/claude-pantheon-rs/build-deb.sh            # writes build/claude-pantheon-rs-deb/*.deb
# Needs: cargo, libgtk-4-dev, libgranite-7-dev, meson, valac, libwingpanel-dev, libjson-glib-dev,
# libgee-0.8-dev, librsvg2-bin, dpkg-deb. Set CLAUDE_PANTHEON_SKIP_INDICATOR=1 to package the app alone.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"
out="${CLAUDE_PANTHEON_OUTPUT_DIR:-$repo_root/build/claude-pantheon-rs-deb}"
stage="$out/package"
id=io.github.techmore.ClaudePantheonRs

for tool in cargo dpkg-deb rsvg-convert; do
  command -v "$tool" >/dev/null || { echo "Missing build tool: $tool" >&2; exit 2; }
done
with_indicator=1
if [[ ${CLAUDE_PANTHEON_SKIP_INDICATOR:-0} == 1 ]]; then
  with_indicator=0
else
  for tool in meson ninja valac; do
    command -v "$tool" >/dev/null || { echo "Missing build tool: $tool (set CLAUDE_PANTHEON_SKIP_INDICATOR=1 to skip the indicator)" >&2; exit 2; }
  done
fi

version="$(grep -m1 -oP '^version = "\K[0-9.]+' "$here/Cargo.toml")"
arch="$(dpkg --print-architecture)"
debver="${version}-1~mseries2"

rm -rf "$out"
mkdir -p "$stage"

# ---- the app ---------------------------------------------------------------------------------
(cd "$here" && cargo build --release --locked 2>/dev/null || cargo build --release)
target_dir="$(cd "$here" && cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
install -Dm755 "$target_dir/release/claude-pantheon-rs" "$stage/usr/bin/claude-pantheon-rs"
install -Dm755 "$target_dir/release/claude-pantheon-sdk" "$stage/usr/bin/claude-pantheon-sdk"
install -Dm755 "$repo_root/mseries/install-tstack-skills.py" "$stage/usr/bin/mseries-install-tstack-skills"
for skill in full-send full-stop council sol-loop file-pr takeover babysit-pr modernize leaving-pr-comments; do
  install -d "$stage/usr/share/mseries/tstack/$skill"
  cp -a "$repo_root/.agents/skills/$skill/." "$stage/usr/share/mseries/tstack/$skill/"
done
install -Dm644 "$repo_root/.agents/skills/TSTACK-LICENSE.txt" "$stage/usr/share/mseries/tstack/TSTACK-LICENSE.txt"
install -Dm755 "$repo_root/mseries/fleet-shepherd/bin/mseries-fleet" "$stage/usr/bin/mseries-fleet"
install -Dm755 "$repo_root/mseries/fleet-shepherd/bin/fleet-snapshot" "$stage/usr/lib/mseries/fleet/fleet-snapshot"
install -Dm644 "$repo_root/mseries/fleet-shepherd/LICENSE" "$stage/usr/share/doc/claude-pantheon-rs/fleet-shepherd-MIT.txt"
install -Dm755 "$repo_root/mseries/repobar.py" "$stage/usr/bin/mseries-repobar"
install -Dm644 "$here/data/io.github.techmore.ClaudePantheonSdk.desktop" "$stage/usr/share/applications/io.github.techmore.ClaudePantheonSdk.desktop"
install -Dm644 "$here/data/$id.desktop" "$stage/usr/share/applications/$id.desktop"
install -Dm644 "$here/data/$id.metainfo.xml" "$stage/usr/share/metainfo/$id.metainfo.xml"
install -d "$stage/usr/share/dbus-1/services"
sed 's|@BINDIR@|/usr/bin|' "$here/data/$id.service" > "$stage/usr/share/dbus-1/services/$id.service"
install -Dm644 "$here/data/$id.svg" "$stage/usr/share/icons/hicolor/scalable/apps/$id.svg"
install -Dm644 "$here/data/$id-symbolic.svg" "$stage/usr/share/icons/hicolor/symbolic/apps/$id-symbolic.svg"
for size in 16 24 32 48 64 128; do
  install -d "$stage/usr/share/icons/hicolor/${size}x${size}/apps"
  rsvg-convert -w "$size" -h "$size" "$here/data/$id.svg" -o "$stage/usr/share/icons/hicolor/${size}x${size}/apps/$id.png"
done

# ---- the Wingpanel indicator -------------------------------------------------------------------
if ((with_indicator)); then
  work="$out/indicator"
  meson setup "$work" "$here/../../mseries/t3-wingpanel" --prefix=/usr --buildtype=release >/dev/null
  meson compile -C "$work"
  DESTDIR="$stage" meson install -C "$work" >/dev/null
  install -Dm755 "$repo_root/mseries/activate-t3-panel.sh" "$stage/usr/bin/mseries-activate-t3-panel"
fi
find "$stage" -type d -exec chmod 0755 {} +
find "$stage/usr/share" -type f -exec chmod 0644 {} +

# ---- control -----------------------------------------------------------------------------------
installed_kb="$(du -sk "$stage" | cut -f1)"
install -d "$stage/DEBIAN"
depends="libgtk-4-1 (>= 4.20), libgranite-7-7 (>= 7.8) | libgranite7 (>= 7.8), libglib2.0-0, libgtksourceview-5-0, libvte-2.91-gtk4-0, libwebkitgtk-6.0-4, librsvg2-common, secret-tool | libsecret-tools, gh, git, python3"
if ((with_indicator)); then
  depends="$depends, libjson-glib-1.0-0, libgee-0.8-2, wingpanel | io.elementary.wingpanel"
fi
cat >"$stage/DEBIAN/control" <<CONTROL
Package: claude-pantheon-rs
Version: $debver
Section: devel
Priority: optional
Architecture: $arch
Installed-Size: $installed_kb
Depends: $depends
Recommends: xdg-desktop-portal
Maintainer: elementaryos-mseries local build
Homepage: https://github.com/techmore/ElementaryOS_MSeries
Description: Native elementary OS client of the T3 Code server (Claude Code, Codex, OpenCode, Grok)
 One window drives Claude Code, Codex, OpenCode and Grok threads through the
 T3 Code server, with the local Claude Code transcripts as the offline view:
 pairing, streamed turns, permission prompts, usage with plan windows,
 repos and issues, diagnostics. Includes the Wingpanel indicator.
CONTROL

cat >"$stage/DEBIAN/postinst" <<'POST'
#!/bin/sh
set -e
command -v update-desktop-database >/dev/null && update-desktop-database -q /usr/share/applications || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
exit 0
POST
cp "$stage/DEBIAN/postinst" "$stage/DEBIAN/postrm"
chmod 0755 "$stage/DEBIAN/postinst" "$stage/DEBIAN/postrm"

pkg="$out/claude-pantheon-rs_${debver}_${arch}.deb"
dpkg-deb --root-owner-group --build "$stage" "$pkg" >/dev/null
echo "Built $pkg"
dpkg-deb --info "$pkg" | sed -n '1,12p'
dpkg-deb --contents "$pkg" | awk '{print $1, $3, $6}'
