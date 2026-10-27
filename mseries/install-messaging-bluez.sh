#!/usr/bin/env bash
# Scoped, reversible BlueZ service installation. Does not replace distro files.
set -euo pipefail
prefix=/usr/local/libexec/mseries-bluez-5.87
system_drop=/etc/systemd/system/bluetooth.service.d/91-mseries-bluez-587.conf
user_root=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
runtime_root=${XDG_RUNTIME_DIR:?}/systemd/user
action=${1:---status}
if [[ $action == --status ]]; then
    systemctl show bluetooth.service -p ExecStart
    systemctl --user show obex.service blueferry.service -p ExecStart -p Environment
    exit
fi
[[ $EUID != 0 ]] || { echo 'Run as the desktop user; sudo is used only for system files.' >&2; exit 1; }
if [[ $action == --install ]]; then
    source_dir=${2:?Usage: install-messaging-bluez.sh --install BLUEZ_5_87_BUILD_DIR}
    [[ $(git -C "$source_dir" rev-parse HEAD) == 65d11edee469c24dcc8d075b8be9c040cebaaf98 ]] || {
        echo 'Expected the reviewed official BlueZ 5.87 commit.' >&2; exit 1;
    }
    [[ $("$source_dir/src/bluetoothd" --version) == 5.87 ]]
    [[ $("$source_dir/client/bluetoothctl" --version) == *5.87* ]]
    for binary in src/bluetoothd obexd/src/obexd client/bluetoothctl; do
        test -x "$source_dir/$binary"
        if ldd "$source_dir/$binary" | rg -q 'not found'; then
            echo "Missing library for $binary" >&2; exit 1
        fi
    done
    sudo install -d -m755 "$prefix" "$(dirname "$system_drop")"
    for binary in src/bluetoothd obexd/src/obexd client/bluetoothctl; do
        sudo install -o root -g root -m755 "$source_dir/$binary" "$prefix/${binary##*/}"
    done
    printf '[Service]\nExecStart=\nExecStart=%s/bluetoothd --experimental\n' "$prefix" |
        sudo tee "$system_drop" >/dev/null
    mkdir -p "$user_root/obex.service.d" "$user_root/blueferry.service.d"
    printf '[Service]\nExecStart=\nExecStart=%s/obexd\n' "$prefix" > "$user_root/obex.service.d/91-mseries-bluez-587.conf"
    # Upstream BlueFerry probes bluetoothctl's version to choose pairing mode.
    # Keep that tool matched to the running daemon in this service's scope.
    printf '[Service]\nEnvironment=PATH=%s:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\n' "$prefix" > "$user_root/blueferry.service.d/91-mseries-bluez-587.conf"
    sudo rm -f /run/systemd/system/bluetooth.service.d/91-mseries-bluez-587-trial.conf
    rm -f "$runtime_root/obex.service.d/91-mseries-bluez-587-trial.conf" "$runtime_root/blueferry.service.d/91-mseries-bluez-587-trial.conf"
    sudo systemctl daemon-reload
    systemctl --user daemon-reload
    # Persist an already-running successful trial without interrupting messages.
    # On another host, explicitly use --restart after reviewing the installation.
    echo 'Installed persistent scoped configuration; running services were not restarted.'
elif [[ $action == --restart ]]; then
    systemctl --user stop blueferry.service obex.service
    sudo systemctl restart bluetooth.service
    systemctl --user start obex.service blueferry.service
elif [[ $action == --rollback ]]; then
    systemctl --user stop blueferry.service obex.service
    sudo rm -f "$system_drop" /run/systemd/system/bluetooth.service.d/91-mseries-bluez-587-trial.conf
    rm -f "$user_root/obex.service.d/91-mseries-bluez-587.conf" "$user_root/blueferry.service.d/91-mseries-bluez-587.conf" "$runtime_root/obex.service.d/91-mseries-bluez-587-trial.conf" "$runtime_root/blueferry.service.d/91-mseries-bluez-587-trial.conf"
    sudo systemctl daemon-reload
    systemctl --user daemon-reload
    sudo systemctl restart bluetooth.service
    systemctl --user start obex.service blueferry.service
    echo 'Distro services restored. Current phone pairing and contacts retained.'
else
    echo 'Use --status, --install BUILD_DIR, --restart, or --rollback.' >&2
    exit 2
fi
