#!/bin/sh
# Build and install on request without touching an existing running client.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
prefix=${1:-"$HOME/.local"}
case "$prefix" in /*) ;; *) echo 'Installation prefix must be an absolute path' >&2; exit 2;; esac
if [ -d "$project_dir/build" ]; then
    meson configure "$project_dir/build" --prefix="$prefix"
else
    meson setup "$project_dir/build" "$project_dir" --prefix="$prefix"
fi
meson compile -C "$project_dir/build"
meson test -C "$project_dir/build" --print-errorlogs
meson install -C "$project_dir/build" --no-rebuild
printf 'Installed Signal Pantheon under %s. Runtime: Java 25 and signal-cli 0.14.9.\n' "$prefix"
