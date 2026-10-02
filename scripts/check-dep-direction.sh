#!/usr/bin/env sh
# Dependency direction check (handoff-arch.md §1): arrows go one way.
# Reads `cargo metadata` (or a saved copy via METADATA=<file>) and fails on any
# workspace-internal edge not in the allow list below.
#
#   arch-facts    → (nothing of ours)
#   arch-analyze  → facts
#   arch-views    → facts
#   arch-forge    → facts
#   arch-driver   → facts
#   arch-session  → facts, views, driver, forge
#   arch-api      → facts, analyze, views, session, forge, driver
#   arch-cli      → api
set -eu
cd "$(dirname "$0")/.."

allowed() {
  case "$1" in
    arch-facts) echo "" ;;
    arch-analyze|arch-views|arch-forge|arch-driver) echo "arch-facts" ;;
    arch-session) echo "arch-facts arch-views arch-driver arch-forge" ;;
    arch-api) echo "arch-facts arch-analyze arch-views arch-session arch-forge arch-driver" ;;
    arch-cli) echo "arch-api" ;;
    *) echo "UNKNOWN" ;;
  esac
}

if [ -n "${METADATA:-}" ]; then meta=$(cat "$METADATA"); else meta=$(cargo metadata --format-version 1 --no-deps); fi

edges=$(printf '%s' "$meta" | jq -r '
  [.packages[].name] as $ws
  | .packages[] | .name as $from
  | .dependencies[] | select(.name as $n | $ws | index($n)) | "\($from) \(.name)"')

status=0
count=0
for pkg in $(printf '%s' "$meta" | jq -r '.packages[].name'); do
  case "$(allowed "$pkg")" in UNKNOWN) echo "FAIL: $pkg is not in the allow list; add it to scripts/check-dep-direction.sh"; status=1 ;; esac
done
printf '%s\n' "$edges" | while read -r from to; do
  [ -z "$from" ] && continue
  ok=0
  for a in $(allowed "$from"); do [ "$a" = "$to" ] && ok=1; done
  if [ "$ok" = 1 ]; then echo "ok    $from -> $to"; else echo "FAIL  $from -> $to (reverse or undeclared dependency)"; exit 1; fi
done || status=1
[ "$status" = 0 ] && echo "dependency direction: ok" || { echo "dependency direction: FAILED"; exit 1; }
