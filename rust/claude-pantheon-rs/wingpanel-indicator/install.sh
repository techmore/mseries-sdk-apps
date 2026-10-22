#!/usr/bin/env bash
# Build and install the Claude Wingpanel indicator.
#
#   ./install.sh              build, run the probe, install into Wingpanel's indicator dir (sudo); the panel
#                             loads it at the next login (the owner's menu bar is never restarted by default)
#   ./install.sh --restart    the same, then restart Wingpanel now (owner-requested reloads only)
#   ./install.sh --uninstall
#
# Wingpanel 8/9 loads indicators only from `pkg-config --variable=indicatorsdir wingpanel-9`
# (/usr/lib/<triplet>/wingpanel-9 here); there is no per-user indicator directory, so the copy needs sudo.
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
indicators_dir=$(pkg-config --variable=indicatorsdir wingpanel-9)
icons=/usr/share/icons/hicolor
id=io.github.techmore.ClaudePantheonRs

restart=0
for arg in "$@"; do [[ $arg == --restart ]] && restart=1; done

reload_panel() {
    if ((restart)) && systemctl --user is-active --quiet io.elementary.wingpanel 2>/dev/null; then
        systemctl --user restart io.elementary.wingpanel && echo "Restarted Wingpanel."
    else
        echo "Staged: Wingpanel loads it at the next login (or run with --restart)."
    fi
}

if [[ ${1:-} == --uninstall ]]; then
    sudo rm -f "$indicators_dir/libclaude-pantheon.so" "$icons/symbolic/apps/$id-symbolic.svg"
    sudo gtk-update-icon-cache -q -f "$icons" 2>/dev/null || true
    echo "Removed the Claude indicator."
    reload_panel
    exit 0
fi

for tool in meson ninja valac pkg-config; do
    command -v "$tool" >/dev/null || {
        echo "Missing $tool. Install build requirements with:" >&2
        echo '  sudo apt install meson ninja-build valac libwingpanel-dev libgranite-7-dev libjson-glib-dev libgee-0.8-dev' >&2
        exit 1
    }
done

build_dir=$(mktemp -d "${XDG_RUNTIME_DIR:-/tmp}/claude-indicator.XXXXXX")
trap 'rm -rf "$build_dir"' EXIT
meson setup "$build_dir" "$here" --prefix=/usr >/dev/null
ninja -C "$build_dir"
if command -v xvfb-run >/dev/null; then
    (cd "$here" && xvfb-run -a env GDK_BACKEND=x11 GSETTINGS_BACKEND=memory GSK_RENDERER=cairo "$build_dir/probe" probe/status-fixture.json)
fi
sudo ninja -C "$build_dir" install
# The panel icon: the app's symbolic icon in the system theme, where Wingpanel looks.
if [[ -f "$here/../data/$id-symbolic.svg" ]]; then
    sudo install -Dm644 "$here/../data/$id-symbolic.svg" "$icons/symbolic/apps/$id-symbolic.svg"
    sudo gtk-update-icon-cache -q -f "$icons" 2>/dev/null || true
fi
echo "Installed the Claude indicator to $indicators_dir."
reload_panel
