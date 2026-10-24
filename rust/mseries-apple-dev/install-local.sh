#!/usr/bin/env bash
# Install Apple Dev for the current user (~/.local), no sudo. PROFILE=debug installs the debug build.
#   rust/mseries-apple-dev/install-local.sh
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
id=org.techmore.mseries.AppleDev
profile="${PROFILE:-release}"
flag=(); [[ $profile == release ]] && flag=(--release)
(cd "$here/.." && cargo build "${flag[@]}" -p mseries-apple-dev)
target_dir="$(cd "$here/.." && cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
install -Dm755 "$target_dir/$profile/mseries-apple-dev" "$HOME/.local/bin/mseries-apple-dev"
sed "s|^Exec=mseries-apple-dev|Exec=$HOME/.local/bin/mseries-apple-dev|" "$here/data/$id.desktop" >"$data/applications/$id.desktop"
install -Dm644 "$here/data/$id.metainfo.xml" "$data/metainfo/$id.metainfo.xml"
install -Dm644 "$here/data/$id.svg" "$data/icons/hicolor/scalable/apps/$id.svg"
install -Dm644 "$here/data/$id-symbolic.svg" "$data/icons/hicolor/symbolic/apps/$id-symbolic.svg"
gtk-update-icon-cache -q -t -f "$data/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$data/applications" 2>/dev/null || true
echo "Installed Apple Dev ($profile) to ~/.local. Open it from the applications menu."
