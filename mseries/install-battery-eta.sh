#!/usr/bin/env bash
# Show the time-to-full estimate and the live battery power draw (watts) beside the native Wingpanel battery icon,
# and add a "Battery History…" button to its popover that opens the Battery app. `--build-only` builds and checks
# without installing or restarting anything.
set -euo pipefail

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
# shellcheck source=mseries/lib-backup.sh
source "$repo/mseries/lib-backup.sh"
commit=e05723d8cba82b596a5efc259b4e0d15ad9a2bc4
target_dir=$(pkg-config --variable=indicatorsdir wingpanel-9)
target="$target_dir/libpower.so"
backup=/usr/local/lib/mseries/libpower.so.before-charge-eta

if [[ ${1:-} == --uninstall ]]; then
  if [[ ! -f $backup ]]; then
    echo "No saved Power indicator at $backup" >&2
    exit 1
  fi
  mseries_install_indicator 0644 "$backup" "$target"
  sudo rm -f "$backup"
  python3 "$repo/mseries/local-ai/repair-panel.py"
  echo 'Restored the original elementary Power indicator.'
  exit 0
fi

for tool in git meson ninja valac msgfmt pkg-config; do
  command -v "$tool" >/dev/null || {
    echo "Missing $tool. Install build requirements with:" >&2
    echo '  sudo apt install git meson ninja-build valac gettext libgtop2-dev libudev-dev libwingpanel-9-dev libgranite-7-dev libgtk-4-dev libnotify-dev' >&2
    exit 1
  }
done

installed_version=$(dpkg-query -W -f='${Version}' io.elementary.panel.power 2>/dev/null || true)
if [[ $installed_version != 8.0.2* ]]; then
  echo "This patch targets elementary Power indicator 8.0.2; found ${installed_version:-unknown}." >&2
  exit 1
fi

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/mseries-battery-eta.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT
source_dir="$work_dir/source"
build_dir="$work_dir/build"
stage_dir="$work_dir/stage"

git init -q "$source_dir"
git -C "$source_dir" remote add origin https://github.com/elementary/panel-power.git
git -C "$source_dir" fetch -q --depth=1 origin "$commit"
git -C "$source_dir" checkout -q --detach FETCH_HEAD
# Order matters: the watt meter patch is written against the source with the ETA patch applied.
for patch in panel-power-charge-eta.patch panel-power-watts.patch panel-power-watts-toggle.patch panel-power-battery-history.patch panel-power-level-gauge.patch panel-power-gauge-percentage.patch; do
  git -C "$source_dir" apply "$repo/mseries/patches/$patch"
done

source "$repo/mseries/lib-build.sh" 2>/dev/null && mseries_ccache_enable || true   # optional ccache (#47)
meson setup "$build_dir" "$source_dir" --prefix=/usr --buildtype=release
ninja -C "$build_dir" >/dev/null
DESTDIR="$stage_dir" meson install -C "$build_dir" >/dev/null

staged_module="$stage_dir$target"
if [[ ! -f $staged_module ]] || ldd "$staged_module" | grep -q 'not found'; then
  echo 'The staged Power indicator is missing or has unresolved libraries.' >&2
  exit 1
fi

if [[ ${1:-} == --build-only ]]; then
  echo "Built and checked $staged_module; nothing installed, Wingpanel untouched."
  exit 0
fi

if [[ ! -f $backup ]]; then
  sudo install -D -m 0644 "$target" "$backup"
fi
mseries_install_indicator 0644 "$staged_module" "$target"
python3 "$repo/mseries/local-ai/repair-panel.py"
echo 'Installed the native battery ETA, watt meter, Battery History button and level-aware battery gauge. Wingpanel shows the live power draw (and the time to full while charging).'
