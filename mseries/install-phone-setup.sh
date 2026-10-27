#!/usr/bin/env bash
set -euo pipefail
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
command -v blueferry >/dev/null || { echo 'Install BlueFerry backend and GTK client first.' >&2; exit 1; }
python3 -c 'import gi, dbus, PIL; gi.require_version("Gtk", "4.0"); gi.require_version("Granite", "7.0"); gi.require_version("GdkPixbuf", "2.0"); gi.require_version("Secret", "1")'
python3 "$here/phone-setup/test_setup.py"
python3 "$here/phone-setup/test_protocol.py"
python3 "$here/phone-setup/test_connection.py"
python3 "$here/phone-setup/test_messages.py"
python3 "$here/phone-setup/test_message_media.py"
python3 "$here/phone-setup/test_message_pins.py"
root=${XDG_DATA_HOME:-$HOME/.local/share}/mseries/phone-setup
install -d "$root" "$HOME/.local/bin" "${XDG_DATA_HOME:-$HOME/.local/share}/applications"
install -m644 "$here/phone-setup/setup_core.py" "$root/setup_core.py"
install -m644 "$here/phone-setup/connection.py" "$root/connection.py"
install -m755 "$here/phone-setup/phone_status.py" "$root/phone_status.py"
install -m755 "$here/phone-setup/phone-setup.py" "$root/phone-setup.py"
install -m755 "$here/phone-setup/messages.py" "$root/messages.py"
install -m644 "$here/phone-setup/messages_model.py" "$root/messages_model.py"
install -m644 "$here/phone-setup/messages_style.py" "$root/messages_style.py"
install -m644 "$here/phone-setup/messages_settings.py" "$root/messages_settings.py"
install -m644 "$here/phone-setup/messages_widgets.py" "$root/messages_widgets.py"
install -m644 "$here/phone-setup/message_media.py" "$root/message_media.py"
install -m644 "$here/phone-setup/message_pins.py" "$root/message_pins.py"
icons=${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps
install -d "$icons"
install -m644 "$here/phone-setup/io.elementary.mseries.Messages.svg" "$icons/io.elementary.mseries.Messages.svg"
icon_theme=${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor
if [[ ! -f "$icon_theme/index.theme" ]]; then
    install -m644 /usr/share/icons/hicolor/index.theme "$icon_theme/index.theme"
fi
gtk4-update-icon-cache -f "$icon_theme" >/dev/null 2>&1 || true
# Pairing capability checks must use the client matching our scoped BlueZ service.
for pair in 'mseries-phone-setup:phone-setup.py' 'mseries-messages:messages.py'; do
    launcher=${pair%%:*}
    entry=${pair#*:}
    python3 - "$HOME/.local/bin/$launcher" <<'PY'
from pathlib import Path
import sys
Path(sys.argv[1]).unlink(missing_ok=True)
PY
    cat > "$HOME/.local/bin/$launcher" <<LAUNCHER
#!/bin/sh
if [ -f /etc/systemd/system/bluetooth.service.d/91-mseries-bluez-587.conf ]; then
    PATH=/usr/local/libexec/mseries-bluez-5.87:\$PATH
    export PATH
fi
exec /usr/bin/python3 "$root/$entry" "\$@"
LAUNCHER
    chmod 755 "$HOME/.local/bin/$launcher"
done
ln -sfn "$root/phone_status.py" "$HOME/.local/bin/mseries-phone-status"
cat > "${XDG_DATA_HOME:-$HOME/.local/share}/applications/io.elementary.mseries.PhoneSetup.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Connect iPhone
Comment=Set up iPhone messages on this desktop
Exec="$HOME/.local/bin/mseries-phone-setup"
Icon=phone
Categories=Network;Settings;
Terminal=false
DESKTOP
cat > "${XDG_DATA_HOME:-$HOME/.local/share}/applications/io.elementary.mseries.Messages.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Messages
Comment=Send and receive texts through your iPhone
Exec="$HOME/.local/bin/mseries-messages"
Icon=io.elementary.mseries.Messages
Categories=Network;Chat;
Keywords=iPhone;iMessage;SMS;BlueFerry;
TryExec=$HOME/.local/bin/mseries-messages
StartupNotify=true
Terminal=false
DESKTOP
update-desktop-database "${XDG_DATA_HOME:-$HOME/.local/share}/applications" 2>/dev/null || true
printf '%s\n' 'Messages and Connect iPhone installed. Launch mseries-messages.'
