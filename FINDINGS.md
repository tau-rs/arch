# FINDINGS · from:arch

Findings made while building `tau-rs/arch` that change a design decision. The record of each is
the `from:arch` issue in `tau-rs/arch-design`; this file keeps the evidence next to the experiment
that produced it.

| finding | arch-design issue | arch issue |
|---|---|---|
| F-1 / F-2 | https://github.com/tau-rs/arch-design/issues/15 | #2 |
| F-3 | https://github.com/tau-rs/arch-design/issues/16 | #10 |

## F-1 · Hooks passed via `--settings` never fire under `claude -p --bare` (2026-10-02)

**Affects:** ADR 12 (confinement), HANDOFF §2 `arch-driver` command line, milestone 5.

**Summary.** The driver was designed to spawn `claude -p --bare … --settings {hooks}` and rely on
PreToolUse/PostToolUse hooks for the stale-write guard, the element-scope veto and attribution.
Under `--bare` those hooks do not run, so an agent would have no confinement at all. The CLI
documents this itself: the string inside the binary reads *"Present only under --bare /
CLAUDE_CODE_SIMPLE with the hooks surface gated off: settings-file, flag, policy, and plugin hooks
never fire there; session hooks still run."* Dropping `--bare` and restricting the setting sources
instead gives the isolation we wanted **and** working hooks, verified empirically below.

**Evidence** (`experiments/hooks-under-bare.sh`, Claude Code 2.1.272, macOS, model haiku):

| run | PreToolUse/PostToolUse markers | deny (exit 2) blocks the tool | notes |
|---|---|---|---|
| plain + `--settings hooks` | fired | yes, model sees "blocked by a hook with the error: …" | baseline |
| `--bare` + `--settings hooks` | did not run | n/a | no hook events at all; also see F-2 |
| `--bare` + `--settings` SessionStart/UserPromptSubmit hooks | did not run | n/a | pre-model hooks, so independent of auth |
| no `--bare`, `--setting-sources ''`, `--strict-mcp-config`, `--settings hooks` | fired | yes | user/project/local settings not loaded; only our hooks and MCP |

**Decision proposed (needs an ADR 12 amendment).** The claude-code adapter invokes:

```
claude -p --output-format stream-json --verbose --include-hook-events
       --setting-sources '' --strict-mcp-config --no-session-persistence
       --settings <arch hooks + permissions> --mcp-config <arch MCP server>
       --append-system-prompt-file <context pack> --allowedTools … 
       --permission-mode acceptEdits --permission-prompts none
```

Why: `--setting-sources ''` plus `--strict-mcp-config` removes the user's own settings, hooks and
MCP servers from the agent's run, which is what `--bare` was chosen for; hooks given by flag still
load. One honest consequence: CLAUDE.md auto-discovery, LSP, plugin sync and auto-memory are not
disabled by this combination; `--bare`'s other switches (`CLAUDE_CODE_DISABLE_CLAUDE_MDS`, etc.)
can be set per switch if a later finding shows they matter. The alternative that keeps `--bare`,
"session hooks" over the SDK control protocol (`--input-format stream-json`, `hook_callback`
requests), is more code in the adapter and still has F-2's billing constraint.

## F-2 · `--bare` accepts only API-key auth; OAuth (claude.ai / Max plan) cannot drive it (2026-10-02)

**Affects:** ADR 12, ADR 23 (secrets), who can use the delegated session at all.

Under `--bare` the CLI reads no OAuth credentials: not the keychain, not
`CLAUDE_CODE_OAUTH_TOKEN` in the environment. `ANTHROPIC_API_KEY` or an `apiKeyHelper` in
`--settings` are the only paths. An OAuth access token fed through either path is rejected with
`401 API key is invalid`. So a driver built on `--bare` would force every arch user onto API-key
billing, and a user logged in through claude.ai could never run a session. The F-1 command line
(no `--bare`) uses the user's normal login. Evidence: the same script, runs B and D.

## F-3 · The 21 link kinds are not reconstructible from the documents handed over (2026-10-02)

**Affects:** `arch-facts` `Link.kind` enum, `schemas/facts.schema.json`, arch-fixtures golden facts.

HANDOFF §2 says "the 21 kinds in four families + refers-to" and names 19: calls · calls port ·
depends on port · implements · refines · inherits · uses type · holds · reads · matches on · tests ·
re-exports · expands · decorates · constructs (resolved) and hands off · listens to · calls out ·
wires (guessed). Spec §7 adds two relations described as facts but not named as kinds:
`route → handler` with middleware, and `shared table used as a queue`. The schema ships with those
two as `routes` and `queues`, which makes 21, in four families chosen by what the link is about:

| family | kinds |
|---|---|
| call | calls · calls-port · depends-on-port · hands-off · listens-to · calls-out · wires · routes |
| type | implements · refines · inherits · uses-type · holds · constructs · matches-on |
| data | reads (flag `access: read\|write`) · queues |
| structure | tests · re-exports · expands · decorates |

plus `refers-to` outside the families. The schema is versioned (`schema_version`); renaming or
regrouping before the first golden facts are pinned costs nothing, after it costs a fixture bump.
**Needs confirmation** against the arch-design source for the list.
