#!/usr/bin/env bash
# Install PolyBagger Print Manager for this user.
# The app is copied into ~/.local/share so the launcher keeps working after a reboot,
# a branch switch, or a moved/deleted checkout. Re-run this script to update it.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
app_id=io.techmore.PolyBaggerPrintManager
dest="${XDG_DATA_HOME:-$HOME/.local/share}/applications/$app_id.desktop"
mkdir -p "$(dirname "$dest")"
app_dir="${XDG_DATA_HOME:-$HOME/.local/share}/polybagger-print-manager/app"
install -Dm755 "$here/polybagger_print_manager.py" "$app_dir/polybagger_print_manager.py"
install -Dm644 "$here/$app_id.svg" "$app_dir/$app_id.svg"

# app icon: scalable SVG plus PNG sizes for the dock and app grid, in hicolor so every icon theme finds it
icons="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
install -Dm644 "$here/$app_id.svg" "$icons/scalable/apps/$app_id.svg"
if command -v rsvg-convert >/dev/null; then
  for size in 16 24 32 48 64 128 256; do
    mkdir -p "$icons/${size}x${size}/apps"
    rsvg-convert -w "$size" -h "$size" "$here/$app_id.svg" -o "$icons/${size}x${size}/apps/$app_id.png"
  done
fi
gtk-update-icon-cache -q -t -f "$icons" 2>/dev/null || true
cat > "$dest" <<DESKTOP
[Desktop Entry]
Name=PolyBagger Print Manager
Comment=Load labels onto the T-1000 bagger's Zebra printer and verify each step
Exec=python3 "$app_dir/polybagger_print_manager.py"
Icon=$app_id
Terminal=false
Type=Application
Categories=Utility;
StartupWMClass=$app_id
DESKTOP
python3 "$app_dir/polybagger_print_manager.py" --sync >/dev/null || true
update-desktop-database -q "$(dirname "$dest")" 2>/dev/null || true
echo "Installed $app_dir, $dest and the $app_id icon"
