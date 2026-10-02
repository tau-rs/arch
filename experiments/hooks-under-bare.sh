#!/usr/bin/env bash
# Experiment · do Claude Code hooks passed via --settings fire under `claude -p --bare`?
# Why: arch-driver (HANDOFF.md §2, spec §13.12) planned to confine agents with PreToolUse/PostToolUse
# hooks passed by --settings while spawning `claude -p --bare`. Result and consequences: FINDINGS.md F-1, F-2.
#
# Runs (each asks haiku for exactly one Bash tool call that creates a side-effect file):
#   A  plain        + --settings hooks   observe (markers) · deny (PreToolUse exit 2)
#   B  --bare       + --settings hooks   observe · deny      ← the question
#   C  no --bare, --setting-sources '' --strict-mcp-config + --settings hooks   observe · deny  ← the alternative
#   D  --bare, no auth at all (control for the auth finding)
# Under --bare, OAuth/keychain are never read; the script uses ANTHROPIC_API_KEY if set and otherwise
# tries the keychain OAuth access token as CLAUDE_CODE_OAUTH_TOKEN (recorded as a result, never printed).
set -uo pipefail
MODEL="${MODEL:-haiku}"
tmp="$(mktemp -d)"
echo "# hooks under --bare · $(date -u +%Y-%m-%dT%H:%MZ) · $(claude --version 2>&1 | head -1) · model $MODEL"
echo

cat > "$tmp/observe.json" <<JSON
{"hooks":{
 "PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"cat > $tmp/pre.stdin; echo pre >> $tmp/fired"}]}],
 "PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo post >> $tmp/fired"}]}]
}}
JSON
cat > "$tmp/deny.json" <<JSON
{"hooks":{
 "PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo pre >> $tmp/fired; echo 'arch: denied by experiment' >&2; exit 2"}]}]
}}
JSON

bare_env=()
if [ -n "${ANTHROPIC_API_KEY:-}" ]; then
  bare_auth="ANTHROPIC_API_KEY (env)"
elif tok="$(security find-generic-password -s 'Claude Code-credentials' -w 2>/dev/null | python3 -c 'import sys,json;print(json.load(sys.stdin)["claudeAiOauth"]["accessToken"])' 2>/dev/null)" && [ -n "$tok" ]; then
  bare_env=(env "CLAUDE_CODE_OAUTH_TOKEN=$tok")
  bare_auth="CLAUDE_CODE_OAUTH_TOKEN (keychain OAuth token, env)"
else
  bare_auth="none"
fi
echo "auth offered to --bare runs: $bare_auth"
echo

run() { # name mode(observe|deny) flags...
  local name="$1" mode="$2"; shift 2
  rm -f "$tmp/fired" "$tmp/pre.stdin" "$tmp/side"
  : > "$tmp/fired"
  local -a pre=(); case "$name" in B*) pre=("${bare_env[@]}");; esac
  "${pre[@]}" claude -p --model "$MODEL" --output-format stream-json --verbose --include-hook-events \
    --settings "$tmp/$mode.json" --allowedTools Bash --permission-mode acceptEdits --permission-prompts none "$@" \
    "Use the Bash tool to run exactly: touch $tmp/side ; then reply with the single word done." \
    > "$tmp/$name.jsonl" 2> "$tmp/$name.err" < /dev/null
  local rc=$?
  local fired; fired="$(paste -sd, "$tmp/fired")"
  local hooks; hooks="$(grep -o '"hook_name":"[^"]*"' "$tmp/$name.jsonl" | sort | uniq -c | awk '{printf "%s×%s ", $2, $1}')"
  local side; [ -e "$tmp/side" ] && side=yes || side=no
  local res; res="$(grep '"type":"result"' "$tmp/$name.jsonl" | python3 -c 'import sys,json
for l in sys.stdin:
    d=json.loads(l); print(("error: " if d.get("is_error") else "ok: ")+str(d.get("result"))[:80].replace("|","/").replace("\n"," ")); break' 2>/dev/null)"
  local src; src="$(grep -o '"apiKeySource":"[^"]*"' "$tmp/$name.jsonl" | head -1 | cut -d: -f2)"
  echo "| $name | $mode | $rc | ${fired:-—} | ${hooks:-—} | $side | ${src:-—} | ${res:-—} |"
}

echo "| run | hooks | exit | markers written by hooks | hook events in stream | Bash side effect | apiKeySource | result |"
echo "|---|---|---|---|---|---|---|---|"
run "A plain" observe
run "A plain" deny
run "B bare" observe --bare
run "B bare" deny --bare
run "C no-bare, setting-sources ''" observe --setting-sources '' --strict-mcp-config
run "C no-bare, setting-sources ''" deny --setting-sources '' --strict-mcp-config
echo
echo "D · --bare with no credentials at all:"
env -u ANTHROPIC_API_KEY -u CLAUDE_CODE_OAUTH_TOKEN claude -p --bare --model "$MODEL" --settings "$tmp/observe.json" "say ok" > "$tmp/D.out" 2>&1 < /dev/null
echo "  exit $? · $(grep -v 'no stdin data' "$tmp/D.out" | head -c 200 | tr '\n' ' ')"
echo
echo "The binary's own string for this state (strings \$(which claude) | grep 'hooks surface gated off'):"
LC_ALL=C /usr/bin/grep -a -o 'Present only under --bare / CLAUDE_CODE_SIMPLE with the hooks surface gated off[^"]*' "$(readlink -f "$(which claude)")" | head -1 | sed 's/^/  /'
echo
echo "raw logs: $tmp"
