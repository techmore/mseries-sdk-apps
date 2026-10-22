#!/usr/bin/env bash
# refresh-contracts.sh [--check] [--record]
#
# Refreshes the sparse clone of pingdotgg/t3code (T3_CONTRACTS_REPO, default ~/claude-work/repos/t3code),
# extracts ORCHESTRATION_PROTOCOL_VERSION and the RPC method lists from packages/contracts/src into
# contracts/PROTOCOL_VERSION and contracts/METHODS (what build.rs compiles in), and with --record
# re-records tests/fixtures from the running server (`claude-pantheon-rs --t3-dump`).
# --check only compares upstream with the vendored copy and exits 2 on drift (CI uses this).
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
repo="${T3_CONTRACTS_REPO:-$HOME/claude-work/repos/t3code}"
check=0; record=0
for a in "$@"; do case "$a" in --check) check=1 ;; --record) record=1 ;; *) echo "usage: $0 [--check] [--record]" >&2; exit 1 ;; esac; done

if [ ! -d "$repo/.git" ]; then
  echo "cloning the contracts (sparse, blobless) into $repo" >&2
  git clone --quiet --filter=blob:none --sparse https://github.com/pingdotgg/t3code.git "$repo"
  git -C "$repo" sparse-checkout set packages/contracts
else
  git -C "$repo" fetch --quiet origin main
  git -C "$repo" checkout --quiet -B main origin/main
fi
src="$repo/packages/contracts/src"
commit="$(git -C "$repo" rev-parse --short HEAD)"
date="$(git -C "$repo" log -1 --format=%cs)"

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
grep -oE 'ORCHESTRATION_PROTOCOL_VERSION = [0-9]+' "$src/environment.ts" | grep -oE '[0-9]+$' > "$tmp/PROTOCOL_VERSION"
{
  echo "# RPC tags of pingdotgg/t3code packages/contracts (WS_METHODS + ORCHESTRATION_V2_WS_METHODS)"
  echo "# from commit $commit ($date); regenerate with scripts/refresh-contracts.sh"
  for pair in "rpc.ts:WS_METHODS" "orchestrationV2.ts:ORCHESTRATION_V2_WS_METHODS"; do
    f="${pair%%:*}"; name="${pair##*:}"
    awk -v name="$name" '$0 ~ "export const " name " = \\{" {on=1; next} on && /^\}/ {exit} on' "$src/$f" \
      | grep -oE ':\s*"[A-Za-z][A-Za-z0-9.-]*"' | grep -oE '"[^"]+"' | tr -d '"'
  done | sort -u
} > "$tmp/METHODS"

if ((check)); then
  drift=0
  for f in PROTOCOL_VERSION METHODS; do
    if ! diff -u <(grep -v '^#' "$here/contracts/$f") <(grep -v '^#' "$tmp/$f"); then drift=1; fi
  done
  if ((drift)); then echo "contracts drift against t3code $commit; run scripts/refresh-contracts.sh" >&2; exit 2; fi
  echo "contracts match t3code $commit"
  exit 0
fi
cp "$tmp/PROTOCOL_VERSION" "$tmp/METHODS" "$here/contracts/"
echo "contracts/: protocol $(cat "$here/contracts/PROTOCOL_VERSION"), $(grep -vc '^#' "$here/contracts/METHODS") methods from t3code $commit"
if ((record)); then
  echo "recording tests/fixtures from the running server" >&2
  (cd "$here/.." && cargo run --release -q -p claude-pantheon-rs -- --t3-dump "$here/tests/fixtures")
fi
