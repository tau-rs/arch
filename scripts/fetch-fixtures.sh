#!/usr/bin/env sh
# Clone tau-rs/arch-fixtures at the commit pinned in fixtures/pin.toml into fixtures/arch-fixtures/
# (gitignored). CI runs this before `cargo test`; run it locally once, and again after a pin bump.
set -eu
cd "$(dirname "$0")/.."
repo=$(sed -n '/^\[arch-fixtures\]/,/^\[/p' fixtures/pin.toml | sed -n 's/^repo *= *"\([^"]*\)".*/\1/p')
commit=$(sed -n '/^\[arch-fixtures\]/,/^\[/p' fixtures/pin.toml | sed -n 's/^commit *= *"\([^"]*\)".*/\1/p')
dest=fixtures/arch-fixtures
if [ -z "$commit" ]; then echo "fixtures/pin.toml: arch-fixtures commit is empty; nothing to fetch"; exit 0; fi
if [ -d "$dest/.git" ] && [ "$(git -C "$dest" rev-parse HEAD)" = "$commit" ]; then echo "arch-fixtures already at $commit"; exit 0; fi
rm -rf "$dest"
git init -q "$dest"
git -C "$dest" remote add origin "$repo"
git -C "$dest" fetch -q --depth 1 origin "$commit"
git -C "$dest" checkout -q FETCH_HEAD
echo "arch-fixtures at $commit"
