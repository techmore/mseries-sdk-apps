#!/bin/sh
set -eu
fixture_root=$(mktemp -d)
trap 'rm -rf "$fixture_root"' EXIT HUP INT TERM
export XDG_DATA_HOME="$fixture_root/data"
export XDG_CACHE_HOME="$fixture_root/cache"
export XDG_RUNTIME_DIR="$fixture_root/runtime"
mkdir -m 700 "$XDG_RUNTIME_DIR"
export SIGNAL_CLI_PATH="$1"
exec_binary=$2
"$exec_binary"
