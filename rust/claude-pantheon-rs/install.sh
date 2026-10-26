#!/usr/bin/env bash
# Build claude-pantheon-rs and install it for the current user (default) or system-wide.
#
#   ./install.sh                 release build, install under ~/.local (binary, .desktop, D-Bus service,
#                                metainfo, hicolor icons 16/24/32/48/64/128 + scalable + symbolic)
#   ./install.sh --prefix /usr   same, under /usr (needs sudo for the copy)
#   ./install.sh --uninstall     remove everything the script installed
#   ./install.sh --no-build      install what is already built
#
# Stages code without restarting the Dock or client. Use --reload-dock explicitly
# when a newly installed icon needs a Dock reload.
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
prefix="$HOME/.local"
build=1
uninstall=0
reload_dock=0
while (($#)); do
    case "$1" in
        --prefix) prefix=$2; shift 2 ;;
        --prefix=*) prefix=${1#--prefix=}; shift ;;
        --no-build) build=0; shift ;;
        --uninstall) uninstall=1; shift ;;
        --reload-dock) reload_dock=1; shift ;;
        *) echo "unknown option $1" >&2; exit 2 ;;
    esac
done

id=io.github.techmore.ClaudePantheonRs
bindir="$prefix/bin"
datadir="$prefix/share"
apps="$datadir/applications"
icons="$datadir/icons/hicolor"
sudo=()
if [[ ! -w "$prefix" && ! -w "$(dirname "$prefix")" ]]; then
    sudo=(sudo)
fi

if ((uninstall)); then
    "${sudo[@]}" rm -f "$bindir/claude-pantheon-rs" "$bindir/claude-pantheon-sdk" "$apps/$id.desktop" \
        "$apps/io.github.techmore.ClaudePantheonSdk.desktop" "$datadir/dbus-1/services/$id.service" \
        "$datadir/metainfo/$id.metainfo.xml" "$icons/scalable/apps/$id.svg" "$icons/symbolic/apps/$id-symbolic.svg"
    for size in 16 24 32 48 64 128; do
        "${sudo[@]}" rm -f "$icons/${size}x${size}/apps/$id.png"
    done
    "${sudo[@]}" gtk-update-icon-cache -q -f "$icons" 2>/dev/null || true
    "${sudo[@]}" update-desktop-database "$apps" 2>/dev/null || true
    echo "Removed Claude from $prefix."
    exit 0
fi

# ---- build -----------------------------------------------------------------------------------
if ((build)); then
    (cd "$here" && cargo build --release)
fi
# The shared target dir from ~/.cargo/config.toml, else the crate's own.
target_dir=$(cd "$here" && cargo metadata --format-version 1 --no-deps 2>/dev/null | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
binary="$target_dir/release/claude-pantheon-rs"
[[ -x "$binary" ]] || { echo "no release binary at $binary; run without --no-build" >&2; exit 1; }
[[ -x "$target_dir/release/claude-pantheon-sdk" ]] || { echo 'SDK release binary is missing; run without --no-build' >&2; exit 1; }

# ---- install ---------------------------------------------------------------------------------
"${sudo[@]}" install -Dm755 "$binary" "$bindir/claude-pantheon-rs"
"${sudo[@]}" install -Dm755 "$target_dir/release/claude-pantheon-sdk" "$bindir/claude-pantheon-sdk"
"${sudo[@]}" install -Dm644 "$here/data/io.github.techmore.ClaudePantheonSdk.desktop" "$apps/io.github.techmore.ClaudePantheonSdk.desktop"
# Shared read-only dependency: also used by the optional RepoBar indicator.
"${sudo[@]}" install -Dm755 "$here/../../mseries/repobar.py" "$bindir/mseries-repobar"
"${sudo[@]}" install -Dm644 "$here/data/$id.desktop" "$apps/$id.desktop"
"${sudo[@]}" install -Dm644 "$here/data/$id.metainfo.xml" "$datadir/metainfo/$id.metainfo.xml"
"${sudo[@]}" install -d "$datadir/dbus-1/services"
sed "s|@BINDIR@|$bindir|" "$here/data/$id.service" | "${sudo[@]}" tee "$datadir/dbus-1/services/$id.service" >/dev/null
"${sudo[@]}" install -Dm644 "$here/data/$id.svg" "$icons/scalable/apps/$id.svg"
"${sudo[@]}" install -Dm644 "$here/data/$id-symbolic.svg" "$icons/symbolic/apps/$id-symbolic.svg"
if command -v rsvg-convert >/dev/null; then
    tmp=$(mktemp -d "${XDG_RUNTIME_DIR:-/tmp}/claude-pantheon-icons.XXXXXX")
    trap 'rm -rf "$tmp"' EXIT
    for size in 16 24 32 48 64 128; do
        rsvg-convert -w "$size" -h "$size" "$here/data/$id.svg" -o "$tmp/$size.png"
        "${sudo[@]}" install -Dm644 "$tmp/$size.png" "$icons/${size}x${size}/apps/$id.png"
    done
else
    echo "rsvg-convert not found; only the scalable icon was installed (install librsvg2-bin for the PNG sizes)" >&2
fi
"${sudo[@]}" gtk-update-icon-cache -q -f "$icons" 2>/dev/null || true
"${sudo[@]}" update-desktop-database "$apps" 2>/dev/null || true

# ---- Pantheon quirks -------------------------------------------------------------------------
if [[ "$prefix" == "$HOME/.local" ]]; then
    case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "note: $HOME/.local/bin is not on PATH; the .desktop entry still works through D-Bus activation" ;; esac
fi
if ((reload_dock)) && systemctl --user is-active --quiet pantheon-dock 2>/dev/null; then
    systemctl --user restart pantheon-dock && echo "Restarted pantheon-dock so the new icon shows."
fi
echo "Installed Claude to $prefix (binary, desktop entry with New Thread and Usage actions, D-Bus activation, metainfo, icons)."
