#!/usr/bin/env bash
# Install Tailscale for the current user (~/.local), no sudo. PROFILE=debug installs the debug build.
#   rust/mseries-tailscale/install-local.sh
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
id=org.techmore.mseries.Tailscale
profile="${PROFILE:-release}"
flag=(); [[ $profile == release ]] && flag=(--release)
(cd "$here/.." && cargo build "${flag[@]}" -p mseries-tailscale)
target_dir="$(cd "$here/.." && cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
install -Dm755 "$target_dir/$profile/mseries-tailscale" "$HOME/.local/bin/mseries-tailscale"
mkdir -p "$data/applications"
sed "s|^Exec=mseries-tailscale|Exec=$HOME/.local/bin/mseries-tailscale|" "$here/data/$id.desktop" >"$data/applications/$id.desktop"
install -Dm644 "$here/data/$id.svg" "$data/icons/hicolor/scalable/apps/$id.svg"
for icon in "$id-symbolic" mseries-tailscale-symbolic mseries-tailscale-offline-symbolic; do
  install -Dm644 "$here/data/$icon.svg" "$data/icons/hicolor/symbolic/apps/$icon.svg"
done
# The old launcher opened Tailscale's web page in a browser; this app replaces it.
old="$data/applications/tailscale-control.desktop"
if [[ -f $old ]]; then
  mkdir -p "$data/applications/.replaced" && mv -f "$old" "$data/applications/.replaced/"
fi
gtk-update-icon-cache -q -t -f "$data/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$data/applications" 2>/dev/null || true
echo "Installed Tailscale ($profile) to ~/.local. Open it from the applications menu."
