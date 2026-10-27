#!/bin/bash
# Actual generated executable; private desktops and synthetic state only.
set -euo pipefail
sdk_matrix_binary=$(realpath "$1")
sdk_matrix_output=$2
sdk_matrix_probe=$(realpath "$(dirname "$0")/font-profile.py")
test ! -e "$sdk_matrix_output"
mkdir -p "$sdk_matrix_output"
sdk_matrix_output=$(realpath "$sdk_matrix_output")
for sdk_matrix_theme in light dark; do
  sdk_matrix_gtk_theme=Adwaita
  if [ "$sdk_matrix_theme" = dark ]; then sdk_matrix_gtk_theme=Adwaita:dark; fi
  for sdk_matrix_text in 1 1.5; do
    for sdk_matrix_width in 480 900; do
      sdk_matrix_file="$sdk_matrix_output/$sdk_matrix_theme-text$sdk_matrix_text-width$sdk_matrix_width.png"
      sdk_matrix_points=10
      if [ "$sdk_matrix_text" = 1.5 ]; then sdk_matrix_points=15; fi
      sdk_matrix_config="$sdk_matrix_output/config-$sdk_matrix_theme-$sdk_matrix_text-$sdk_matrix_width"
      mkdir -p "$sdk_matrix_config/gtk-4.0"
      printf '[Settings]\ngtk-font-name=Sans %s\n' "$sdk_matrix_points" > "$sdk_matrix_config/gtk-4.0/settings.ini"
      MSDK_MATRIX_PROBE="$sdk_matrix_probe" MSDK_MATRIX_POINTS="$sdk_matrix_points" MSDK_MATRIX_BIN="$sdk_matrix_binary" MSDK_MATRIX_FILE="$sdk_matrix_file" \
      MSDK_MATRIX_WIDTH="$sdk_matrix_width" XDG_DATA_HOME="$sdk_matrix_output/state" \
      XDG_CONFIG_HOME="$sdk_matrix_config" GTK_THEME="$sdk_matrix_gtk_theme" \
      GDK_SCALE=1 GSK_RENDERER=cairo \
      xvfb-run -a dbus-run-session -- bash -eu -c '
        python3 "$MSDK_MATRIX_PROBE"
        "$MSDK_MATRIX_BIN" --screenshot "$MSDK_MATRIX_FILE" &
        sdk_matrix_pid=$!
        trap '\''kill "$sdk_matrix_pid" 2>/dev/null || true'\'' EXIT
        sdk_matrix_window=$(timeout 8 xdotool search --sync --name "^SDK Settings$" | head -1)
        xdotool windowsize --sync "$sdk_matrix_window" "$MSDK_MATRIX_WIDTH" 400
        wait "$sdk_matrix_pid"
        trap - EXIT
      '
      test -s "$sdk_matrix_file"
    done
  done
done
