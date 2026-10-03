#!/usr/bin/env sh
# Milestone 4 proof (issue #5): clone zero2prod at the commit pinned in fixtures/pin.toml,
# run `arch init` then `arch check`, and pass on the exit code of `arch check`:
# 0 nothing blocks, 1 a blocking finding, 2 a tool error. Before checking, the areas.toml init
# wrote must equal fixtures/expected/zero2prod/areas.toml (ADR 0029), on two runs. Writes the
# JSON report to $ARCH_REPORT (default target/zero2prod-check.json). Run it locally the same way
# CI does.
set -eu
cd "$(dirname "$0")/.."
root=$(pwd)
repo=$(sed -n '/^\[repos.zero2prod\]/,/^\[/p' fixtures/pin.toml | sed -n 's/^repo *= *"\([^"]*\)".*/\1/p')
commit=$(sed -n '/^\[repos.zero2prod\]/,/^\[/p' fixtures/pin.toml | sed -n 's/^commit *= *"\([^"]*\)".*/\1/p')
if [ -z "$repo" ] || [ -z "$commit" ]; then echo "fixtures/pin.toml: [repos.zero2prod] needs repo and commit"; exit 2; fi
report=${ARCH_REPORT:-$root/target/zero2prod-check.json}

cargo build -q -p arch-cli
arch=$root/target/debug/arch

# A fresh clone every run: `arch init` refuses a repository that already has .arch/ (ADR 0029).
dest=$(mktemp -d)/zero2prod
trap 'rm -rf "$(dirname "$dest")"' EXIT
git init -q "$dest"
git -C "$dest" remote add origin "$repo"
git -C "$dest" fetch -q --depth 1 origin "$commit"
git -C "$dest" checkout -q FETCH_HEAD
echo "zero2prod at $commit"

# `arch init` makes one commit (ADR 0006); a CI runner has no git identity of its own.
GIT_AUTHOR_NAME=${GIT_AUTHOR_NAME:-arch-ci} GIT_AUTHOR_EMAIL=${GIT_AUTHOR_EMAIL:-arch-ci@localhost} \
GIT_COMMITTER_NAME=${GIT_COMMITTER_NAME:-arch-ci} GIT_COMMITTER_EMAIL=${GIT_COMMITTER_EMAIL:-arch-ci@localhost} \
  "$arch" init "$dest"
if [ "$(git -C "$dest" rev-list --count FETCH_HEAD..HEAD)" != 1 ] || [ -n "$(git -C "$dest" status --porcelain)" ]; then
  echo "arch init did not leave exactly one commit and a clean tree"; exit 2
fi

# ADR 0029, "Result on zero2prod": the written areas.toml, byte for byte, and the same bytes from a
# second run on a clean copy of the checkout.
expected=$root/fixtures/expected/zero2prod/areas.toml
if ! diff -u "$expected" "$dest/.arch/areas.toml"; then
  echo "arch init on zero2prod differs from $expected (ADR 0029)"; exit 2
fi
again=$(dirname "$dest")/again
git clone -q --no-checkout "$dest" "$again"
git -C "$again" checkout -q "$commit"
"$arch" init "$again" --no-commit > /dev/null
if ! cmp -s "$expected" "$again/.arch/areas.toml"; then
  echo "a second arch init on zero2prod wrote different bytes"; exit 2
fi

mkdir -p "$(dirname "$report")"
status=0
"$arch" check "$dest" || status=$?
json_status=0
"$arch" check "$dest" --format json > "$report" || json_status=$?
if [ "$json_status" != "$status" ]; then
  echo "arch check exit code differs between human ($status) and JSON ($json_status) output"; exit 2
fi
echo "report: $report"
exit "$status"
