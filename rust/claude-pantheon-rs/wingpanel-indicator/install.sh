#!/usr/bin/env bash
# Compatibility entrypoint: the shared native T3 menu replaces this module.
set -euo pipefail
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
exec bash "$here/../../../mseries/install-t3-wingpanel.sh" "$@"
