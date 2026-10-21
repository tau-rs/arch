#!/usr/bin/env bash
# Writes golden/<repo>/sizes.json from measured counts. Re-run after a pin bump.
#
# What is measured, and how:
#   commit        git HEAD of the repo (submodule gitlink, or this repo's HEAD for smallsvc)
#   crates        packages from `cargo metadata --no-deps` (falls back to counting [package] tables)
#   rust_files    *.rs outside target/
#   rust_lines    lines in those files
#   decls         syntactic item declarations (fn · struct · enum · trait · type · const · static · mod · union ·
#                 macro_rules) in *.rs outside target/, tests/, benches/, examples/. This is an upper bound on
#                 arch's `items` (it includes #[cfg(test)] modules and non-unit crates); `items` and `links` are
#                 counted from golden/<repo>/facts.json, null until arch-analyze has produced it.
set -euo pipefail
cd "$(dirname "$0")/.."

DECL_RE='^\s*(pub(\([^)]*\))?\s+)?(default\s+)?(const\s+)?(async\s+)?(unsafe\s+)?(extern\s+("[^"]*"\s+)?)?(fn|struct|enum|trait|type|const|static|mod|union)\s+[A-Za-z_]|^\s*macro_rules!\s'

measure() {
  local name=$1 dir=$2
  local commit crates files lines decls
  commit=$(git -C "$dir" rev-parse HEAD)
  if crates=$(cd "$dir" && cargo metadata --no-deps --format-version 1 --offline 2>/dev/null | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["packages"]))'); then :; else
    crates=$(rg -l --glob 'Cargo.toml' --glob '!target' '^\[package\]' "$dir" | wc -l | tr -d ' ')
  fi
  files=$(rg --files --type rust --glob '!target' "$dir" | wc -l | tr -d ' ')
  lines=$(rg --files --type rust --glob '!target' "$dir" | xargs wc -l | tail -1 | awk '{print $1}')
  decls=$( (rg --type rust --glob '!target' --glob '!**/tests/**' --glob '!**/benches/**' --glob '!**/examples/**' -c "$DECL_RE" "$dir" || true) | awk -F: '{s+=$NF} END {print s+0}')
  local by_kind="" sep=""
  for k in fn struct enum trait type const static mod; do
    local n
    n=$( (rg --type rust --glob '!target' --glob '!**/tests/**' --glob '!**/benches/**' --glob '!**/examples/**' -c "^\s*(pub(\([^)]*\))?\s+)?(default\s+)?(const\s+)?(async\s+)?(unsafe\s+)?(extern\s+(\"[^\"]*\"\s+)?)?$k\s+[A-Za-z_]" "$dir" || true) | awk -F: '{s+=$NF} END {print s+0}')
    by_kind="$by_kind$sep\"$k\": $n"; sep=", "
  done
  local commit_json="\"$commit\""
  [ "$name" = smallsvc ] && commit_json=null
  # items and links are read from the golden facts once arch-analyze has produced them.
  local items=null links=null
  if [ -f "golden/$name/facts.json" ]; then
    read -r items links < <(python3 -c 'import json,sys; f=json.load(open(sys.argv[1])); print(len(f.get("items",[])), len(f.get("links",[])))' "golden/$name/facts.json")
  fi
  mkdir -p "golden/$name"
  cat > "golden/$name/sizes.json" <<JSON
{
  "repo": "$name",
  "commit": $commit_json,
  "measured_at": "$(date -u +%Y-%m-%d)",
  "measured_by": "tools/measure-sizes.sh",
  "crates": $crates,
  "rust_files": $files,
  "rust_lines": $lines,
  "decls": $decls,
  "decls_by_kind": { $by_kind },
  "items": $items,
  "links": $links,
  "notes": "crates from cargo metadata --no-deps; decls is a syntactic upper bound on items (see script header; fn includes methods); items and links are filled by arch-analyze; smallsvc is in-tree so its commit is this repo's"
}
JSON
  echo "$name commit=${commit:0:12} crates=$crates files=$files lines=$lines decls=$decls"
}

measure smallsvc  repos/smallsvc
measure ripgrep   repos/ripgrep
measure zero2prod repos/zero2prod
measure zed       repos/zed
