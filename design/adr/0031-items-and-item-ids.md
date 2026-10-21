# ADR 0031 · Items and their ids: what rust-analyzer calls an item, variants and fields named on the link; a method sits under its impl's header, a non-lib target is marked

- Date: 2026-10-03
- Status: accepted
- Source: tau-rs/arch-design#5 (arch-fixtures F-3, decided by the user on tau-rs/arch#14) and tau-rs/arch-design#22 (found while planning arch-analyze, tau-rs/arch#3)

## In plain words

An item is a box the map can draw and an end a link can land on, so two things must be settled before golden facts freeze them: which pieces of code are items, and what each one is called. The first was decided on tau-rs/arch#14: everything rust-analyzer calls an item, methods included, but not enum variants or struct fields, which would fill the map with one box per field; a link that touches a variant or a field lands on its type and names the member on the side. The second is the item id, the name every other fact uses to point at an item: `<crate>::<module path>::<name>#<kind>`, a Rust path with the kind at the end. That rule had two holes, and smallsvc has both: two impl blocks in one module can each define `new`, and a package with a library and a binary has two crate roots with the same name. The answer: a method's id sits under its impl block, and the block is named by its header as written (`impl Outbox for PgOutbox`); the binary's items carry `[bin:<name>]` after the crate name, the library's carry nothing. One analogy: a postal address, where the impl header is the building and the method the flat; a second building with the same name on the same street gets a number. These are the ids arch-analyze already writes and the merged smallsvc golden already holds (286 items, no clash), so neither repo changes.

## Context

- Item kinds (`Item.kind`, tau-rs/arch `docs/arch-facts.md`): `fn · struct · enum · trait · impl · mod · macro · const · static · type-alias · union`.
- tau-rs/arch#22 (merged) added two optional fields to the fact model and `schemas/facts.schema.json`: `Item.parent` (an associated item's impl or trait) and `Link.member` (the variant or field a link touches). `schema_version` stays 0.
- arch-analyze on tau-rs/arch main (`items.rs`, `pass.rs`) writes the ids in the table below; a repeated id within one crate gets `.2`, `.3` in the order the walker meets it.
- arch-fixtures `golden/smallsvc/facts.json` (merged, arch-fixtures#10): 286 items (129 fn with methods, 53 struct, 47 impl, 33 mod, 17 enum, 5 trait, 2 const), 976 links of which 318 carry a `member`; no repeated id; the bin target's items are `orderly[bin:orderly]#mod` and `orderly[bin:orderly]::main#fn`. `sizes.json` counts 237 declarations, a syntactic upper bound that leaves out impl blocks.

## Decision

**What is an item (tau-rs/arch-design#5).** Everything rust-analyzer calls an item, including associated items (methods, associated consts and types), which carry `parent`: the id of their `impl` or `trait`. `impl` blocks are items of kind `impl`, folded under their type on the map. Enum variants and struct fields are not items: a link that touches one (`matches-on`, `holds`, `reads`, `constructs`) targets the type and carries the member's bare name in `Link.member`.

**Item ids (tau-rs/arch-design#22).**

| case | id | smallsvc |
|---|---|---|
| item in the library | `<crate>::<module path>::<name>#<kind>` | `orderly::worker::run_outbox_worker#fn` |
| crate root | `<crate>#mod` | `orderly#mod` |
| impl block | name is `impl <Trait> for <Type>` or `impl <Type>`, as written, runs of whitespace as one space, the `impl<…>` parameters and the `where` clause left out | `orderly::adapters::postgres::outbox::impl Outbox for PgOutbox#impl` |
| associated item of an impl | the impl's id without `#impl`, then `::<name>#<kind>` | `orderly::adapters::postgres::outbox::impl Outbox for PgOutbox::enqueue#fn` |
| associated item of a trait | the trait's id without `#trait`, then `::<name>#<kind>` | `orderly::ports::outbox::Outbox::enqueue#fn` |
| item of another target | `<crate>[<target kind>:<target name>]` in place of `<crate>`; target kind `bin` · `example` · `test` · `bench`; the library carries no marker | `orderly[bin:orderly]::main#fn` |
| an id already taken in the crate | `.2`, `.3`, … after the kind, in source order (out-of-line modules in declaration order) | `c::impl S#impl.2`, a second `impl S` in one module (not in smallsvc) |

- `<crate>` is the name the code spells (the package name with `-` as `_`, as in `use`); `Item.crate` keeps the package name.
- The associated items of a numbered impl keep the plain prefix: the methods of the second `impl S` are `c::impl S::<name>#fn`, and `parent` names the block. The compiler rejects two methods of one name across inherent impls of one type (E0592) and two impls of one trait for one type (E0119), so these ids clash only between `cfg`-exclusive copies, which take `.2` like any repeat.

Why (tau-rs/arch-design#22): an id must come out the same at syntax and at resolved depth ([ADR 0010](0010-degrade.md)) and hold no line number (MAP-1), so it can only be spelled from what the source says where the item sits; these are also the ids in the merged golden, so recording them costs no fixture bump.

## Consequences

- Nothing changes in tau-rs/arch or arch-fixtures; `docs/arch-facts.md` in tau-rs/arch cites this ADR for the grammar at its next sync.
- `spec/vocabulary.md`: the **item** and **link** entries are amended to this ADR.
- One honest consequence: an impl's id follows its header as written, so rewriting `impl std::fmt::Display for OrderId` as `impl fmt::Display for OrderId` renames the block and its methods. Whatever points at them (a plan element's site, an allow, a map position) follows the rename the way any rename is followed ([ADR 0021](0021-element-identity.md): by site, then similarity).
- A second honest consequence: two impls with the same header in one module are told apart by order. Adding an `impl S` above an existing one turns the existing block's id from `impl S#impl` into `impl S#impl.2`. Its methods keep their ids (plain prefix), and blocks are folded under their type, so only the block's own id moves.
- A third: an associated item's id does not always begin with its parent's id (the numbered case). Consumers read `parent`; they never parse the id.
- The bin marker is in ids only: the pages show an item's name and module path, never its id.
- Settled since: what "folded under their type on the map" draws is decided by tau-rs/arch-design#132 (2026-10-21): an open area draws one row per top-level item, an `impl` block folds into its type's row through a new optional field `Item.self_type`, and `private` loose items fold into their module's row ([ADR 0037](0037-open-area-rows.md)).
- Rejected: methods under their type (`…::outbox::PgOutbox::new#fn`, Rust's own path spelling; a trait method would still need `<PgOutbox as Outbox>::enqueue`, two impls' methods share one prefix, and every method id in the merged golden changes); the trait's resolved path in the impl name (`impl core::fmt::Display for OrderId`; survives re-qualifying, but syntax depth cannot compute it, so ids would differ between depths); one item for all same-header impls of a module (nothing renumbers, but an item would need several spans, a schema change); a marker on the library too (`orderly[lib]::…`; symmetric, but every id of the common case grows); a line number or content hash in the id (breaks MAP-1, or renames on every edit); from tau-rs/arch#14, top-level items only (a method call would have no item to land on) and variants and fields as items (the map fills with field boxes).
