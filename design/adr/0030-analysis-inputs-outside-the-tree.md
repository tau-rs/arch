# ADR 0030 · Inputs outside the tree: each one is in the package id; the target is this machine's unless `areas.toml` pins it

- Date: 2026-10-03
- Status: accepted
- Source: tau-rs/arch-design#96 (found while deciding #21); completes [ADR 0002](0002-keys-commit-hash-deltas.md) "the inputs tau-rs/arch-design#96 pins"

## In plain words

Two people opening the same commit should see the same map, and the map is drawn from the facts alone (MAP-1), so the facts must depend on nothing but what arch knows about. ADR 0002 names a file's facts by everything that determined them; some of what rust-analyzer reads is not a file in the tree, though: the platform it analyses for, the Rust toolchain, and the code build scripts generate into `target/`. Today none of these is recorded, so two machines can produce different facts for the same commit and nobody is told. The rule this ADR states: every input from outside the tree goes into the package id (the per-package part of a file's facts key), and writing it into `.arch/` turns it into an input inside the tree. One analogy: a recipe card either names the oven it was baked in, or the house fixes one oven for everyone; the card is never silent. The platform matters most: items do not depend on it (the item walker reads every `#[cfg]` branch and records the condition on the item), but links do, because rust-analyzer resolves nothing in code switched off for the platform it analyses. On a Mac, a `#[cfg(target_os = "linux")] mod epoll` is on the map with no arrows; on Linux it has them. arch analyses for this machine's platform, records the triple, and shows it ("analyzed for aarch64-apple-darwin"); a team that wants one map for everyone adds one line to `areas.toml`.

## Context

- ADR 0002 (#21): a file's facts key is `hash(path · content hash · package id)`; the package id includes "the inputs tau-rs/arch-design#96 pins". Same inputs, same facts.
- What tau-rs/arch loads today (`arch-analyze` `ra.rs` `Session::load`, `cargo.rs`): rust-analyzer's default `CargoConfig` with `set_test: true`, so the host triple, the unit's default features and `cfg(test)` on; the sysroot `rustc --print sysroot` gives when run in the repository root, which honours a `rust-toolchain.toml`; build scripts' `OUT_DIR` from `cargo check` (`load_out_dirs_from_check`), reusing `target/` as ADR 0026 requires; proc-macros expanded by the sysroot's proc-macro server.
- The item walker keeps every `cfg` branch and records its predicate in the item's `flags.cfg`. For the files rust-analyzer loads, guessed links are dropped and only the links rust-analyzer resolves are kept; it resolves nothing in inactive code.
- `.arch/areas.toml` already carries unit settings: `main_bin` ([ADR 0007](0007-unit-in-v1.md)).

## Decision

| input | in the tree, keyed, or pinned | what goes into the package id | where it is shown |
|---|---|---|---|
| target platform | keyed: this machine's triple; pinned when `areas.toml` sets `target` | the triple | status line: `analyzed for aarch64-apple-darwin`; with a pin, `analyzed for x86_64-unknown-linux-gnu (areas.toml)` |
| Cargo features | in the tree: always the unit's default features, as `Cargo.toml` declares them | nothing new (`Cargo.toml` is in the package's git tree id) | — |
| `cfg(test)` | fixed rule: always on | nothing (the rule is part of the analyzer version) | — |
| Rust toolchain / sysroot | keyed: the toolchain picked in the repository root; pinned by a committed `rust-toolchain.toml` | `rustc -vV` release and commit hash | Checks tab, with the analyzer version |
| build-script `OUT_DIR` | keyed by content | the content hashes of the files under each unit package's `OUT_DIR` | — |
| proc-macro output | determined by `Cargo.lock`, the toolchain and the host, all already keyed | nothing new | — |
| analyzer version and depth | already keyed (ADR 0002) | unchanged | unchanged |

- **The pin.** `.arch/areas.toml` gains `target = "<triple>"`, next to `main_bin`. `arch init` does not write it ([ADR 0006](0006-arch-init.md)): writing the initialiser's triple would make every teammate on another platform cross-compile.
- **The check.** `same_inputs_same_facts`, in arch's CI on the fixture pins: a cold analysis of the same commit in two checkouts at different paths, each with its own `target/` and cache, on the macOS and the Linux runner, with the target pinned to one triple, gives byte-identical facts. `incremental_equals_cold` (ADR 0002) checks incremental against cold on one machine; this one checks cold against cold across machines.

Why (tau-rs/arch-design#96): the key must name everything that determined the facts or two machines read each other's cache entries as valid, and analysing for this machine keeps the first index on the `target/` the developer already built (ADR 0026), where a pinned foreign triple would cost a cross `cargo check` on every other platform.

## Consequences

- `arch-analyze` adds the triple, the `rustc -vV` release and commit hash, and the `OUT_DIR` content hashes to the package id, and passes `areas.toml`'s `target` to rust-analyzer when set. `arch-facts` owns `areas.toml`'s `target` field and records the triple in the facts (`Analyzer`), so the status line can show it.
- One honest consequence: without a pin, a Mac and a Linux machine on the same commit disagree about links inside platform-gated code, so a finding there can block in Linux CI and not show on the Mac. Both say which platform they analysed for; a team that wants one answer pins `target`, and pays the cross `cargo check` (`rustup target add`; `-sys` crates that do not cross-build degrade per [ADR 0010](0010-degrade.md)).
- A second honest consequence: a proc-macro that reads something outside the tree (`sqlx::query!` connecting to `DATABASE_URL`) can change its output with nothing in the key changing. Not covered; environments cannot be enumerated.
- Rejected: `arch init` pinning the initialiser's triple (one map always, but a cross `cargo check` and `rustup target add` for every teammate on another platform, outside ADR 0026's first-index assumption, and degraded crates where `-sys` crates do not cross-build); `--all-features` (features that exclude each other break the load); keying the build-script inputs instead of their output (a build script can read environment variables, system libraries or the network).
