#!/bin/sh
# Installs the importer (command line and progress window) for the current user.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$here"
cargo build --release --locked --features gui
target=${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}
install -Dm755 "$target/release/mseries-signal-import" "$HOME/.local/bin/mseries-signal-import"
install -Dm755 "$target/release/mseries-signal-import-gui" "$HOME/.local/bin/mseries-signal-import-gui"
install -Dm644 data/io.github.sdolbec.SignalHistoryImport.desktop "$HOME/.local/share/applications/io.github.sdolbec.SignalHistoryImport.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$HOME/.local/share/applications" || true
echo 'Installed. Open "Import Signal History" or run mseries-signal-import-gui.'
