#!/usr/bin/env bash
# Record stream-json fixtures for arch-driver's parser and replay double (tau-rs/arch#44).
# Every run uses the adapter's command line (FINDINGS F-1; arch-design#33 option A: --session-id,
# no --no-session-persistence) against a scratch repo, on a small model.
#
#   text         one reply, no tools
#   tools        Read then Edit, with observing PreToolUse/PostToolUse hooks (hook events in the stream)
#   denied       an Edit refused by a PreToolUse hook (exit 2 + reason)
#   subagent     a Task sub-agent
#   resume       --resume of the `text` session with a follow-up message
#   structured   --json-schema, structured output in the result
#   max-turns    --max-turns 1 on a task that needs more: an error result
#
# Output: crates/arch-driver/tests/fixtures/<name>.jsonl, with the scratch path replaced by /work,
# $HOME by /home/user, and session/message/request ids replaced by stable placeholders.
# Usage: experiments/record-stream.sh [out-dir]   (needs a Claude Code login; costs a few cents)
set -euo pipefail
MODEL="${MODEL:-haiku}"
here="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$here/crates/arch-driver/tests/fixtures}"
mkdir -p "$out"
work="$(mktemp -d)"
raw="$work/raw"; mkdir -p "$raw"
repo="$work/repo"; mkdir -p "$repo/src"
printf 'pub fn greet() -> &'"'"'static str {\n    "hello"\n}\n' > "$repo/src/lib.rs"
printf 'notes\n' > "$repo/NOTES.md"
git -C "$repo" init -q && git -C "$repo" add -A && git -C "$repo" -c user.name=t -c user.email=t@t commit -qm init

cat > "$work/observe.json" <<JSON
{"hooks":{
 "PreToolUse":[{"matcher":"Edit|Write","hooks":[{"type":"command","command":"cat > /dev/null; exit 0"}]}],
 "PostToolUse":[{"matcher":"Read|Edit|Write","hooks":[{"type":"command","command":"cat > /dev/null; exit 0"}]}]
}}
JSON
cat > "$work/deny.json" <<JSON
{"hooks":{
 "PreToolUse":[{"matcher":"Edit|Write","hooks":[{"type":"command","command":"cat > /dev/null; echo 'arch: src/lib.rs is outside element a3f9c2e1 (may write: NOTES.md)' >&2; exit 2"}]}]
}}
JSON
echo '{"mcpServers":{}}' > "$work/mcp.json"
echo "You are a test agent. Do exactly what is asked, nothing more." > "$work/pack.md"

run() { # name settings session-flag... -- prompt
  local name="$1" settings="$2"; shift 2
  local -a extra=()
  while [ "$1" != "--" ]; do extra+=("$1"); shift; done; shift
  (cd "$repo" && claude -p --model "$MODEL" \
    --setting-sources '' --strict-mcp-config \
    --output-format stream-json --verbose --include-hook-events \
    --settings "$work/$settings.json" --mcp-config "$work/mcp.json" \
    --append-system-prompt-file "$work/pack.md" \
    --allowedTools Read Edit Write Glob Grep Task \
    --permission-mode acceptEdits --permission-prompts none \
    "${extra[@]}" "$1" > "$raw/$name.jsonl" 2> "$raw/$name.err" < /dev/null) || true
  echo "$name: $(wc -l < "$raw/$name.jsonl") lines, last type $(tail -1 "$raw/$name.jsonl" | python3 -c 'import sys,json;d=json.loads(sys.stdin.read());print(d.get("type"),d.get("subtype"))' 2>/dev/null || echo '?')"
}

text_id="$(uuidgen | tr 'A-Z' 'a-z')"
run text observe --session-id "$text_id" -- "Reply with exactly: ready"
run tools observe --session-id "$(uuidgen | tr 'A-Z' 'a-z')" -- "Read src/lib.rs, then use Edit to change \"hello\" to \"hi\" in it. Then reply done."
git -C "$repo" checkout -q -- .
run denied deny --session-id "$(uuidgen | tr 'A-Z' 'a-z')" -- "Read src/lib.rs, then use Edit to change \"hello\" to \"hi\" in it. If the edit is refused, do not retry: reply with the refusal reason."
run subagent observe --session-id "$(uuidgen | tr 'A-Z' 'a-z')" -- "Use the Task tool with a general-purpose sub-agent to count the files under src/. Then reply with the number."
run resume observe --resume "$text_id" -- "Now reply with exactly: again"
run structured observe --session-id "$(uuidgen | tr 'A-Z' 'a-z')" \
  --json-schema '{"type":"object","properties":{"verdict":{"type":"string","enum":["pass","fail"]},"reason":{"type":"string"}},"required":["verdict","reason"]}' \
  -- "Judge: does src/lib.rs define a function named greet? Answer with the structured verdict."
run max-turns observe --session-id "$(uuidgen | tr 'A-Z' 'a-z')" --max-turns 1 -- "Read src/lib.rs, then read NOTES.md, then reply with both contents."

# Scrub: paths, home, ids. Ids map to stable placeholders per file so the replay stays consistent.
python3 - "$raw" "$out" "$work" "$HOME" <<'PY'
import json, os, re, sys
raw, out, work, home = sys.argv[1:]
real_work = os.path.realpath(work)
uuid = re.compile(r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}')
ids = re.compile(r'\b(msg|req|toolu|srvtoolu)_[0-9A-Za-z]+')
for name in sorted(os.listdir(raw)):
    if not name.endswith('.jsonl'):
        continue
    seen = {}
    def stable(m, kind):
        k = m.group(0)
        if k not in seen:
            seen[k] = f"{kind}-{len([v for v in seen.values() if v.startswith(kind)]) + 1}"
        return seen[k]
    lines = []
    for line in open(os.path.join(raw, name)):
        line = line.rstrip('\n')
        if not line:
            continue
        for w in (real_work, work):
            line = line.replace(w + '/repo', '/work').replace(w, '/scratch')
        line = line.replace(home, '/home/user')
        # Claude Code's per-directory store names the scratch path with every non-alphanumeric as '-'.
        for w in (real_work, work):
            line = line.replace(re.sub(r'[^A-Za-z0-9]', '-', w + '/repo'), '-work')
        line = re.sub(r'/private/tmp/claude-\d+/', '/tmp/claude/', line)
        line = uuid.sub(lambda m: '00000000-0000-4000-8000-' + stable(m, 'u').split('-')[1].zfill(12), line)
        line = ids.sub(lambda m: m.group(1) + '_' + stable(m, m.group(1)).replace('-', ''), line)
        d = json.loads(line)
        # Account usage and thinking signatures are noise for the parser: keep the shape, drop values.
        if d.get('type') == 'rate_limit_event':
            d['rate_limit_info'] = {'status': 'allowed'}
        for block in (d.get('message') or {}).get('content') or []:
            if isinstance(block, dict) and 'signature' in block:
                block['signature'] = 'sig'
        lines.append(json.dumps(d, separators=(',', ':'), ensure_ascii=False))
    open(os.path.join(out, name), 'w').write('\n'.join(lines) + '\n')
    print(f"wrote {os.path.join(out, name)} ({len(lines)} lines)")
PY
echo "raw (unscrubbed) logs: $raw"
