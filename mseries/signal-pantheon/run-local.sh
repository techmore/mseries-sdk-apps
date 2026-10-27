#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# A caller may supply a private runtime bundle without changing the system Java.
runtime_dir=${SIGNAL_PANTHEON_RUNTIME:-"$project_dir/runtime"}
if [ -d "$runtime_dir/tools/bin" ]; then
    export PATH="$runtime_dir/tools/bin:$PATH"
    export SIGNAL_CALL_TUNNEL_BIN="$runtime_dir/tools/bin/signal-call-tunnel"
fi
if [ -d "$runtime_dir/temurin25" ]; then
    export JAVA_HOME="$runtime_dir/temurin25"
    export PATH="$JAVA_HOME/bin:$PATH"
fi
if [ -z "${SIGNAL_CLI_PATH:-}" ] && [ -x "$runtime_dir/signal-cli-arm64/bin/signal-cli" ]; then
    export SIGNAL_CLI_PATH="$runtime_dir/signal-cli-arm64/bin/signal-cli"
fi
if [ -z "${SIGNAL_CLI_PATH:-}" ] && ! command -v signal-cli >/dev/null 2>&1; then
    # Retained development runtime from the original prototype on this host.
    development_tools=/home/sdolbec/Documents/Codex/2026-10-20/claclacladfsfs/work
    if [ -x "$development_tools/signal-cli-arm64/bin/signal-cli" ]; then
        export JAVA_HOME="$development_tools/temurin25"
        export PATH="$JAVA_HOME/bin:$PATH"
        export SIGNAL_CLI_PATH="$development_tools/signal-cli-arm64/bin/signal-cli"
    else
        echo 'Install signal-cli with Java 25, or set SIGNAL_CLI_PATH and JAVA_HOME.' >&2
        exit 1
    fi
fi
if [ "${1:-}" = --diagnose ]; then
    "${SIGNAL_CLI_PATH:-signal-cli}" --version
    "${JAVA_HOME:+$JAVA_HOME/bin/}java" -version
    if [ -x "${SIGNAL_CALL_TUNNEL_BIN:-/nonexistent}" ]; then echo "ARM64 voice tunnel available"; fi
    exit 0
fi
exec "$project_dir/build/signal-pantheon-client" "$@"
