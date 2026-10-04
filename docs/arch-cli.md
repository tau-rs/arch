# arch-cli and arch-api

`arch` is the one binary. It depends on `arch-api` only; `arch-api` holds the methods and
re-exports the types the CLI prints. Live today: `init` and `check` (milestone 4, issue #5),
`hook` and `mcp` (the tool layer, issue #46; see `docs/arch-driver.md`). `serve` answers the app
API's `initialize` on the repo's socket (issue #7); the rest of the method set follows.

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

## `arch serve`

The daemon arch-app talks to (ADR 0034). arch-app runs it with the repo as the current directory
and no argument. The engine then derives its socket from the repo root alone, so the app finds it
without being told:

```
$XDG_RUNTIME_DIR/arch/<hash8>.sock     when XDG_RUNTIME_DIR is set
/tmp/arch-<uid>/<hash8>.sock           otherwise
hash8 = FNV-1a 32 over the repo root's real path (UTF-16 code units), 8 hex digits
```

- **Messages:** one JSON-RPC 2.0 message per line, with one thread per connection. A client is
  ready once `initialize` answers `{ engineVersion, schemaVersion }`. Errors use the standard codes
  (`-32700 · -32600 · -32601 · -32602`).
- **One engine per repo:** if the socket already answers, `arch serve` exits 2 with "already
  serving". A socket file that answers nothing is stale and is replaced.
- **Private directory:** the socket's directory is created with mode `0700`. If it belongs to
  another user or is open to others, `arch serve` refuses to start.
- **Windows:** not supported yet (ADR 0034 §5).

Every transport calls the same handlers: `arch_api::rpc::dispatch`.

## The app API schema: `schemas/arch-api.json`

The contract with arch-app (ADR 0034 in arch-design). It is an OpenRPC 1.3 document generated from
`arch_api::rpc`:

- methods take their params by name;
- results refer to `components.schemas`;
- pushed events are listed in `x-arch-events`.

arch-app pins the file by commit (`tau-rs/arch@<sha>:schemas/arch-api.json`) and generates its
client from it. Nothing is built as a release asset.

`info.version` is semver and starts at `0.1.0`. An addition bumps the minor and a break bumps the
major, even at 0, so a client can tell an engine that added a method from one that broke one.
Every change is announced in arch-design before it ships.

| version | methods | events |
|---|---|---|
| 0.1.0 | `initialize` (`client`, optional `schemaVersion`) → `{ engineVersion, schemaVersion }` | none |

`tests/rpc.rs` checks two things:

- **Drift:** the committed file equals the generated one. `ARCH_UPDATE_SCHEMA=1 cargo test -p
  arch-api` regenerates it.
- **Validity:** the file validates against the OpenRPC 1.3 meta-schema, vendored in
  `crates/arch-api/tests/openrpc/` so the check runs offline.
