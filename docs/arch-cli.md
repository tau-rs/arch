# arch-cli and arch-api

`arch` is the one binary. It depends on `arch-api` only; `arch-api` holds the methods and
re-exports the types the CLI prints. Live today: `init` and `check` (milestone 4, issue #5),
`hook` and `mcp` (the tool layer, issue #46; see `docs/arch-driver.md`).

## `arch init [path] [--no-commit]`

ADR 0006: no questions, one commit.

1. Analyze the repository (syntax-level facts today).
2. Compute the areas, their sides and order (`arch_views::propose_areas`, ADR 0027 to 0029).
3. Write `.arch/areas.toml`, `.arch/rules` (the template: domain must not depend on driving or
   driven; externals only from driven) and the `.arch/cache/` line in `.gitignore`.
4. Commit exactly those files as `chore(arch): add .arch (areas, rules)`.

It refuses a repository that already has `areas.toml` or `rules`: after `init` the files are the
record (ADR 0029). `--no-commit` writes the files and leaves git alone, for a throwaway clone in
CI or to read the result first.

## `arch check [path] [--format human|json]`

Analyze, read `.arch/`, evaluate the dependency rules (`docs/arch-views.md`), print, exit.

| exit code | meaning |
|---|---|
| 0 | nothing blocks: clean, warnings only, or everything allowed |
| 1 | at least one blocking finding |
| 2 | the tool failed (no `.arch/`, unreadable project, bad `areas.toml` pattern) |

`--format json` prints the document described by `schemas/check.schema.json`, generated from
`arch_api::CheckOutput` and drift-tested (`ARCH_UPDATE_SCHEMA=1 cargo test -p arch-api`
regenerates it).

```
arch check · zero2prod @ wt:e3a954cc9 · arch-analyze 0.1.0
note: 1 crate(s) analyzed at syntax level; their findings warn and never block
warn    domain must not depend-on driven
        src/configuration.rs::EmailClientSettings::client → src/email_client.rs::EmailClient::new  (calls, guessed)
        at src/configuration.rs:66
...
42 finding(s): 0 blocking, 42 warning(s), 0 allowed
```

One honest consequence of syntax-level facts: every link is guessed, so every finding is a warning
and the exit code is 0 (ADR 0009). A finding can block once the rust-analyzer pass resolves its
link. The whole tree is checked; scoping to a diff comes later.

## `arch hook pre|post --worktree <w> --session <id> --element <id>`

Claude Code's PreToolUse / PostToolUse / PostToolUseFailure hook (ADR 0012), with the call as JSON
on stdin. The driver's `--settings` file runs it; nobody types it.

| command | exit 0 | exit 2 | exit 1 |
|---|---|---|---|
| `hook pre` | the call goes through | blocked; the reason is on stderr and the model reads it. Also on any failure of arch itself (unknown element, unreadable input): the tool layer fails closed | — |
| `hook post` | recorded | — | arch failed; nothing is blocked |

## `arch mcp --worktree <w> --session <id> --element <id> [--co-author 'Name <email>']`

The MCP server over stdio, tools `read · check · commit · ask`, for one element. The driver's
`--mcp-config` file starts it. `--co-author` defaults to `Claude <noreply@anthropic.com>`.
