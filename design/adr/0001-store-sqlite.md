# ADR 0001 · Store: sqlite in `.arch/cache/`, plain text committed

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.1 (decisions of 2 Oct 2026, taken in the product chat)

## Context

`arch-facts` needs a local store for facts by commit hash, view caches and plan drafts (spec §7, §9). §9 left the choice open between sqlite and sled. `.arch/` also holds content that is committed and read by hand and by agents (areas, rules, allows, sessions).

## Decision

- The store is **sqlite**, under `.arch/cache/`, gitignored.
- Everything committed under `.arch/` is **plain text** (TOML, Markdown, JSONL).
- `arch-facts` owns the sqlite schema.

## Consequences

- `arch-rust` and `arch-views` read and write facts only through `arch-facts`; no other crate opens the database.
- A clone without the cache is complete: the cache is rebuilt from the repo and the committed `.arch/` files.
- `arch init` adds the gitignore line for `.arch/cache/` ([ADR 0006](0006-arch-init.md)).
- The sled option in spec §9 is closed.
