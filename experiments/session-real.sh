#!/usr/bin/env bash
# The opt-in real run of milestone 5 (tau-rs/arch#48, #6): one delegated session end to end with
# real Claude Code (your login) and real GitHub (GITHUB_TOKEN, GH_TOKEN, the keychain or `gh`),
# on a scratch copy of smallsvc. Never run by CI.
#
#   new --delegate  the planner, Accept, every element, the gate (cargo test, arch check, judge)
#   pr              push the branch, open the PR
#   merge           merge with --strategy (default merge), archive to refs/notes/arch,
#                   remove the worktree and the branch
#
# The GitHub repository is created private when it does not exist; delete it afterwards with
# `gh repo delete <owner>/<repo>`. A session that stops for a person (asks, deviation, gate
# failed) stops the script with the command to run next.
#
# Usage: experiments/session-real.sh <owner>/<repo> ["<intention>"]
#   ARCH=<path to arch>       default target/debug/arch (cargo build -p arch-cli first)
#   STRATEGY=merge|squash|rebase   default merge
#   ARCH_MODEL, ARCH_JUDGE_MODEL, ARCH_TEST_COMMAND, ARCH_MAX_TURNS as for `arch session`
set -euo pipefail
slug="${1:?usage: experiments/session-real.sh <owner>/<repo> [\"<intention>\"]}"
intention="${2:-add a refund flow: a Refund in the domain, a port to store refunds, the use case that refunds a paid order, and the in-memory adapter}"
here="$(cd "$(dirname "$0")/.." && pwd)"
arch="${ARCH:-$here/target/debug/arch}"
strategy="${STRATEGY:-merge}"
[ -x "$arch" ] || { echo "no arch at $arch: cargo build -p arch-cli" >&2; exit 2; }
src="$here/fixtures/arch-fixtures/repos/smallsvc"
[ -f "$src/Cargo.toml" ] || { echo "no smallsvc: scripts/fetch-fixtures.sh" >&2; exit 2; }

work="$(mktemp -d)"
repo="$work/${slug#*/}"
echo "== scratch: $repo"
mkdir -p "$repo"
(cd "$src" && tar --exclude=.git --exclude=target --exclude=.arch/cache -cf - .) | (cd "$repo" && tar -xf -)
git -C "$repo" init -q -b main
git -C "$repo" add -A
git -C "$repo" commit -qm "smallsvc, from tau-rs/arch-fixtures"

if gh repo view "$slug" >/dev/null 2>&1; then
  echo "== $slug exists: pushing a fresh main over it"
  git -C "$repo" remote add origin "https://github.com/$slug.git"
  git -C "$repo" push -q --force -u origin main
else
  echo "== creating $slug (private)"
  gh repo create "$slug" --private --source "$repo" --remote origin --push >/dev/null
fi

run() { echo; echo "\$ arch session $*"; "$arch" session --repo "$repo" "$@"; }

out="$(run new "$intention" --delegate | tee /dev/stderr)"
id="$(printf '%s\n' "$out" | sed -n 's/^plan \([0-9a-f]\{8\}\) .*/\1/p' | head -1)"
[ -n "$id" ] || { echo "no session id in the output" >&2; exit 1; }
state="$("$arch" session --repo "$repo" status "$id" --format json | sed -n 's/^  "state": "\(.*\)",$/\1/p')"
if [ "$state" != "done" ]; then
  echo; echo "== the session is $state: settle it, then run pr and merge by hand (--repo $repo)"
  exit 1
fi
run pr "$id"
run merge "$id" --strategy "$strategy"
run status "$id"
echo; echo "\$ git notes --ref arch list"
git -C "$repo" notes --ref arch list
echo; echo "\$ git log --format='%h %s' origin/main"
git -C "$repo" fetch -q origin
git -C "$repo" log --format='%h %s' origin/main | head -12
echo; echo "== done; scratch at $work"
