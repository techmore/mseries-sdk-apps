#!/bin/sh
set -eu
if [ "${SIGNAL_TEST_PRIVATE_BUS:-}" != 1 ]; then
    fixture_root=$(mktemp -d)
    trap 'rm -rf "$fixture_root"' EXIT HUP INT TERM
    export XDG_DATA_HOME="$fixture_root/data" XDG_CACHE_HOME="$fixture_root/cache"
    export XDG_CONFIG_HOME="$fixture_root/config" XDG_RUNTIME_DIR="$fixture_root/runtime"
    mkdir -m 700 "$XDG_RUNTIME_DIR"
    export SIGNAL_TEST_PRIVATE_BUS=1 GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none GIO_USE_VFS=local GSETTINGS_BACKEND=memory
    xvfb-run -a dbus-run-session --config-file="$(dirname "$0")/private-bus.conf" -- "$0" "$@"
    exit $?
fi
# A private test keyring on the private bus. Never connects to the user's keyring.
unset GNOME_KEYRING_CONTROL SSH_AUTH_SOCK GPG_AGENT_INFO
printf 'fixture-only-password' | gnome-keyring-daemon --unlock --components=secrets --control-directory="$XDG_RUNTIME_DIR/keyring" >/dev/null
export SIGNAL_CLI_PATH="$1"
export SIGNAL_TEST_NOTIFICATION_LOG="$XDG_RUNTIME_DIR/notifications.jsonl"
python3 "$(dirname "$0")/notification-service.py" &
notification_pid=$!
trap 'kill "$notification_pid" 2>/dev/null || true' EXIT HUP INT TERM
attempt=0
while [ ! -f "$SIGNAL_TEST_NOTIFICATION_LOG.ready" ]; do
    attempt=$((attempt + 1))
    if [ "$attempt" -gt 50 ]; then echo 'Notification fixture failed to start' >&2; exit 1; fi
    sleep 0.05
done
if [ "${SIGNAL_TEST_GDB:-}" = 1 ]; then
    gdb -batch -ex 'set env G_DEBUG fatal-warnings' -ex run -ex bt --args "$2"
else
    "$2"
fi
