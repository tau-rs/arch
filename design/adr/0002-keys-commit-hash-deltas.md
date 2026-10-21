# ADR 0002 · Keys: facts by commit hash as per-file deltas

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.2 (decisions of 2 Oct 2026, taken in the product chat)
- Settled since: tau-rs/arch-design#21 is decided (2026-10-03): a file's facts are keyed by its path, its content hash and its package's git tree id, not by its content hash alone; a stored or recomputed result is valid only if it equals a cold analysis of the tree, checked by `incremental_equals_cold`; tau-rs/arch-design#96 is decided (2026-10-03): the inputs from outside the tree are named in [ADR 0030](0030-analysis-inputs-outside-the-tree.md)

## Context

Facts must be cheap to recompute for one changed file (< 500 ms, spec §5) and must survive branches, worktrees, uncommitted edits and rebases without a full rebuild. Commits are facts in the V1 data model (spec §3).

## Decision

- Facts are keyed by **commit hash**, stored as **per-file deltas** with content hashes.
- Branches are pointers to a commit's facts.
- Uncommitted work is keyed by a **worktree-state hash**.
- A rebase reuses a file's delta when its facts key matches (below), not on its content hash alone.

  **What names a file's facts.** A file's links are resolved through other files: `src/main.rs` can stay byte-identical while `adapters/postgres/mod.rs` turns `connect` into a re-export, and its `calls` link then points at another item. A delta named by its own content hash alone would be shared by two trees that differ elsewhere, and each would read the other's links marked `resolved`. The rule is: same inputs, same facts. A tree's facts are what a cold analysis of it gives, and a stored or recomputed result is valid only if it is byte-identical to that. A file's facts key is `hash(path · content hash · package id)`. A package (one `Cargo.toml` and its directory) is named the way git names it: its git tree id (`git rev-parse <commit>:<package dir>`; for uncommitted work, the id `git write-tree` would give for the working files), combined with the ids of the unit's packages it depends on, the `Cargo.lock` blob id, the analyzer's version and depth, and the inputs from outside the tree that [ADR 0030](0030-analysis-inputs-outside-the-tree.md) names (the target triple, the toolchain, the build-script output). Packages depend on each other without cycles, so their ids nest like git's trees; files inside a package can depend on each other in cycles, so the package is the smallest node a key can name for links. `tree_files` gains the facts key and `file_facts` is stored under it; `changed_files` keeps comparing content hashes. `arch-facts` owns the columns.

  **Recompute.** Speed lives in the compute step, not in the key. When a save changes function bodies only, `arch-analyze` re-analyses the changed files and carries the package's other deltas forward under the new key; a save that changes a declaration (an item's signature, a `use`, an impl header, a `macro_rules!`, an item nested in a body) re-analyses the package. What a file's facts would need from another file's body is derived when the tree is assembled, not stored in the file's delta: `Table.queue`, and the `queues` link of an inserter, both of which need the dequeuer's `SKIP LOCKED`. `incremental_equals_cold`, in arch's CI on the fixture pins, replays scripted edits and requires the incremental facts to equal a cold analysis byte for byte, for the same edits in any order. Budget ([ADR 0026](0026-budgets-first-index-memory.md)): a body-only save re-analyses about one file and is expected to fit 500 ms with room; a declaration save re-analyses the package, today's cost (457–484 ms on smallsvc from the save, tau-rs/arch#53).

  Why (tau-rs/arch-design#21): the map is a pure function of the facts (MAP-1), so everyone opening the same tree sees the same map only if the facts are named by everything that determined them, as git names its objects; git's own tree id gives that without an arch-specific hash. One honest consequence: every save writes a new delta for each file of the package, so deltas that no recorded tree references any more must be pruned (a cache rule of `arch-facts`). Rejected: keeping the content hash and recomputing the files that link in (two trees sharing a file's bytes overwrite each other's delta, which is what happens today); a key over the files a delta's links were resolved against (the list is gathered during analysis and misses a glob import shadowed by a new item or a new `impl`, so the facts would depend on edit history); items by content hash and links per tree (every tree recomputes all its links); a hash of the package's declarations in the key (fewer deltas, but an arch-specific rule inside the key).

## Consequences

- A worktree's facts are its base commit's facts plus the deltas of the files that differ; the watcher re-analyses only the changed files when a save changes bodies only, and the package when it changes a declaration (spec §6 Sync).
- Two sessions on the same base share facts; concurrent-session overlap ([ADR 0014](0014-concurrent-sessions.md)) is computed from the deltas.
- The schema in `arch-facts` ([ADR 0001](0001-store-sqlite.md)) carries commit, file hash and worktree-state hash as keys from day one, and a facts key per tree file (tau-rs/arch-design#21).
