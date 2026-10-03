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
  `from`, `to`, `kind`, `member`; entries by `item`; commits by history order.
- Item ids never contain line numbers (`<crate>::<module>::<name>#<kind>`), so positions stay
  byte-identical when unrelated lines move (MAP-1).
- Empty flags, empty arrays and absent options are omitted.

### What is an item (arch issue #14, decided 2026-10-03)

Everything rust-analyzer calls an item is an item, **including associated items** (methods,
associated consts and types), which carry `parent`: the id of their `impl` or `trait`. `impl`
blocks are items of kind `impl`, folded under their type on the map. **Enum variants and struct
fields are not items.** A link that touches one (`matches-on`, `holds`, `reads`, `constructs`)
targets the type and names the variant or field in `member`, so "who touches `Status::Paid`" is a
query without the map growing a box per field.

### Decisions encoded (arch-design ADRs)

| ADR | where |
|---|---|
| 2 keys by commit / worktree-state hash | `Repo.commit` (`wt:` prefix for uncommitted trees) |
| 7 one unit, main bin or lib | `Unit.main_target`, `Crate.status` |
| 9 confidence resolved · guessed · declared | `Confidence`, `Link.confidence` |
| 10 degrade to syntax-only facts, reason recorded | `Analyzer.degraded` |
| 16 commits as facts with trailers | `Commit`, `Trailer` |
| 21 content-addressed element ids | `Commit.element` |
| 28 entry kinds: main · framework · spawned worker | `Entry.kind`, `Entry.confidence` |

Open: the link-kind list (FINDINGS F-3, arch-design issue 16, arch issue #10).


## Store (ADR 0001, 0002, 0020)

`crates/arch-facts/src/store/` is the only place SQL lives. `Store::open(<.arch>)` creates
`.arch/cache/facts.sqlite`; the cache is disposable and rebuilt from the repo (a store with an
older `schema_version` in `meta` is dropped and recreated).

| table | what | ADR |
|---|---|---|
| `file_facts` | one file's facts at one content hash (`FileFacts` as JSON): the per-file delta | 0002 |
| `trees`, `tree_files` | a commit or a worktree state → its files at their hashes, plus its `TreeHead` (repo, analyzer, crates, and the facts assembled across files) | 0002, 0007, 0010 |
| `commits`, `tree_commits` | commits as facts, and which are on a tree's branch | 0016 |
| `branches`, `worktrees` | pointers: branch → head; worktree → state hash, base commit, branch | 0002 |
| `view_cache` | per-branch view payloads, tagged with the tree they were computed at | views |
| `plan_drafts` | plans until Accept; discard deletes | 0020 |

The flow the analyzer follows: `put_tree(key, files)` → `missing_file_facts(key)` tells it which
files to compute → `put_file_facts` per file → `facts(key)` assembles the document, sorted per the
stability rules above. `facts` adds the tree's assembled facts (entries, ports, externals,
tables, links derived across files) and sets each item's `entry` flag (an entry names it) and
`reexported` flag (a `re-exports` link points at it), since either may sit in another file. A rebase or a second worktree on the same base needs no recompute: deltas
are shared by file hash. `tests/golden.rs` proves the round trip on `schemas/examples/*.json` and
on every pinned `golden/<repo>/facts.json`.

```mermaid
flowchart LR
  A[analyzer] -- put_tree(key, path→hash) --> S[(store)]
  S -- missing_file_facts --> A
  A -- put_file_facts(FileFacts) --> S
  V[views / check] -- facts(key) --> S
```

## `.arch/` formats (spec §7; ADR 0003, 0004, 0021)

`ArchDir::of_repo(root)` reads and writes the committed files; everything is plain text.

| file | type | content |
|---|---|---|
| `areas.toml` | `Areas` | overrides only: `rule`, `main_bin`, `[[area]] name · paths · side · order` |
| `areas/<name>.md` | text | one description per area, read into the context pack |
| `rules` | `Rules` | TOML: `[[rule]] subject · must_not · targets · level`, `[lints] name = true/false or "block"/"warn"/"off"` (both provisional readings accepted until arch-design#4 decides); `Rules::v1_template()` is ADR 0006 |
| `allows` | `Allows` | TOML: `[[allow]] site · rule · target? · reason · by · at?` |
| `sessions/<id>/session.toml` | `Session` | the session record the scheduler resumes from (ADR 0015): `id · name · state · branch · worktree · base · created`, `[cursor] group · todo · fix_round · extra_rounds`, `[cursor.waiting] element · denied` while an ask or a deviation is open, `[[agents]] element · driver · session_id · transcript_path` (one driver session per element, arch-design#33). Announced in arch-design#117 |
| `sessions/<id>/plan.toml` | `Plan` | elements (`ElementId` = `sha256(session · intention · site)[:8]`, label `E<n>`), groups, gates |
| `sessions/<id>/thread.jsonl` | `ThreadEntry` per line | arch's own thread; driver session id and transcript path as pointers |
| `sessions/<id>/records/NNNN-<kind>.toml` | `Record` | gate-output · judge-verdict · denial · override · resolution, each with witnesses |
| `cache/tool-layer/<element>.json` | `ToolLayerState` | gitignored; JSON: `session · element · expected[{path, sha256}] · read{path: sha256} · writes[{path, sha256, at}]`. arch-driver's hooks and MCP `read` write it under a lock (`ToolLayerFile::update`); the watcher reads `expected` (ADR 0012, arch-design#34) |
| `refs/notes/arch` | `Archive` | at merge, `Archive::move_to_notes` writes the folder as one TOML note and removes it from the tree |

## The arch-fixtures pin

`fixtures/pin.toml` names the arch-fixtures commit; `scripts/fetch-fixtures.sh` clones it into
`fixtures/arch-fixtures/` (gitignored), in CI and locally. `tests/golden.rs` then requires the clone,
reads every `golden/<repo>/sizes.json`, round-trips every `facts.json` present and checks its counts
against `sizes.json`, and parses `repos/smallsvc/.arch/` with the readers above. Bump the pin
deliberately: a golden change is a reviewed PR in arch-fixtures first (handoff §5).

## Open questions filed in arch-design (`from:arch`)

Where an ADR leaves a wire detail open, the code carries the simplest reading and the question is
filed; the product chat decides, then the code follows.

| question | provisional reading in code | issue |
|---|---|---|
| ADR 0021: byte encoding of the element-id hash input | UTF-8 of `session`, `intention`, `site` joined by NUL; sha256 hex; first 8 chars | arch-design#18 |
| ADR 0003: which commit the archive note attaches to, and its encoding | caller passes the merge commit on main; one TOML note with `[[file]] path · content` | arch-design#19 |
| `.arch/rules` and `.arch/allows` shapes, the five lints | TOML as in the table above; lints `god-module · cycle · leaky-port · speculative-abstraction · unresolved-dyn` from the flow pages | arch-design#4 (from:fixtures) |
| the 21 link kinds | `LinkKind` as published | arch-design#16 |
