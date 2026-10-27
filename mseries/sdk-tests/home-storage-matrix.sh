#!/bin/bash
# Generated reference app: real keyboard failure/retry before native capture.
set -euo pipefail
sdk_home_binary=$(realpath "$1")
sdk_home_output=$2
sdk_home_probes=$(realpath "$(dirname "$0")")
test ! -e "$sdk_home_output"
mkdir -p "$sdk_home_output"
sdk_home_output=$(realpath "$sdk_home_output")
for sdk_home_theme in light dark; do
  sdk_home_gtk_theme=Adwaita
  if [ "$sdk_home_theme" = dark ]; then sdk_home_gtk_theme=Adwaita:dark; fi
  for sdk_home_points in 10 15; do
    for sdk_home_width in 480 900; do
      sdk_home_case="$sdk_home_output/$sdk_home_theme-font$sdk_home_points-width$sdk_home_width"
      mkdir -p "$sdk_home_case/config/gtk-4.0"
      printf '[Settings]\ngtk-font-name=Sans %s\n' "$sdk_home_points" > "$sdk_home_case/config/gtk-4.0/settings.ini"
      MSDK_PRIVATE_TEST_BUS=1 MSDK_MATRIX_POINTS="$sdk_home_points" \
      XDG_CONFIG_HOME="$sdk_home_case/config" GTK_THEME="$sdk_home_gtk_theme" GDK_SCALE=1 GSK_RENDERER=cairo \
      xvfb-run -a dbus-run-session -- bash -eu -c '
        python3 "$1/font-profile.py"
        python3 "$1/home-storage-controls.py" --binary "$2" --output "$3/controls.json" \
          --screenshot "$3/recovered.png" --width "$4" --config-home "$3/config"
      ' sdk-home "$sdk_home_probes" "$sdk_home_binary" "$sdk_home_case" "$sdk_home_width" > "$sdk_home_case/session-output.txt" 2>&1
    done
  done
done
