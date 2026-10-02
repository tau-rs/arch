# arch-facts

The fact model, the store and the `.arch/` formats. Depends on nothing else of ours.

## `schemas/facts.schema.json`

The contract between `arch-analyze` (producer), `arch-fixtures` (golden files, reviewed there
first) and `arch-views` (reader). It is **generated from the Rust types** in
`crates/arch-facts/src/model.rs`; the committed file is the published artifact and CI fails when it
drifts (`cargo test -p arch-facts`). Regenerate with:

```
ARCH_UPDATE_SCHEMA=1 cargo test -p arch-facts
```

`schemas/examples/*.json` must validate against the schema and round-trip through the types
byte for byte; they are the smallest examples arch-fixtures can start from.

### Versioning

`schema_version` is `0` until the first golden facts are pinned (issue #9). After that, any change
a reader could notice (a removed field, a renamed enum value, a new required field) bumps it and is
announced in arch-design before it ships (HANDOFF §5). Adding an optional field does not bump.

### Stability rules for golden comparison

- Every array is sorted: items, ports, externals, tables, crates by `id`/`name`; links by
  `from`, `to`, `kind`; entries by `item`; commits by history order.
- Item ids never contain line numbers (`<crate>::<module>::<name>#<kind>`), so positions stay
  byte-identical when unrelated lines move (MAP-1).
- Empty flags, empty arrays and absent options are omitted.

### Decisions encoded (arch-design ADRs)

| ADR | where |
|---|---|
| 2 keys by commit / worktree-state hash | `Repo.commit` (`wt:` prefix for uncommitted trees) |
| 7 one unit, main bin or lib | `Unit.main_target`, `Crate.status` |
| 9 confidence resolved · guessed · declared | `Confidence`, `Link.confidence` |
| 10 degrade to syntax-only facts, reason recorded | `Analyzer.degraded` |
| 16 commits as facts with trailers | `Commit`, `Trailer` |
| 21 content-addressed element ids | `Commit.element` |

Open: the link-kind list (FINDINGS F-3, arch-design issue 16, arch issue #10).

## Store and `.arch/` formats

Second PR of issue #1: sqlite under `.arch/cache/`, per-file deltas keyed by content hash,
branch pointers, `areas.toml` · `rules` · `allows` · `sessions/<id>/` readers and writers, the
`refs/notes/arch` archive, event types.
