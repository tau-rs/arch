# ADR 0032 · `.arch/rules` and `.arch/allows`: TOML, schema version 0 as smallsvc writes it; the five lints are `god-module · cycle · leaky-port · speculative-abstraction · unresolved-dyn`

- Date: 2026-10-04
- Status: accepted
- Source: tau-rs/arch-design#4 (arch-fixtures F-2), decided by the user on tau-rs/arch#13 (2026-10-03); completes [ADR 0006](0006-arch-init.md) "the five lints on"

## In plain words

`.arch/rules` is the file where a team writes what may depend on what, and which lints run; `.arch/allows` is where a person records one exception to a rule, with a reason and a name. The record named both files but gave them no format, and the five lints `arch init` turns on had no names, so tau-rs/arch and arch-fixtures each wrote a provisional version: they agreed on the rules and disagreed on how a lint is switched on and on what the five lints are called. The user decided on tau-rs/arch#13: the shape smallsvc writes is schema version 0 (TOML, one table per rule, a level per lint), and the lint names are the ones arch's template already writes and the earlier design pages printed. One analogy: `rules` is the house rules on the fridge, `allows` the signed notes stuck next to them; a note says which rule it excuses by quoting it, so rewording the rule makes the note stop applying. Nothing changes in arch-fixtures (renamed in arch-fixtures#12); tau-rs/arch makes its template write levels and publishes the two schemas (tau-rs/arch#75).

## Context

- arch-fixtures `repos/smallsvc/.arch/rules` and `allows`: the shape below, lints at `"warn"`, the allow for `NotifyCustomer::deliver → LogNotifier`.
- tau-rs/arch main: `arch-facts` `arch_dir/rules.rs` (`Rules`, `V1_LINTS`, `Rules::v1_template()`, which still writes `name = true`) and `arch_dir/allows.rs`; `arch-views` `findings.rs` matches allows (`site_of`, `allow_covers`).
- The five names appear on `flows/superseded/onboarding-flow.html` ("lints on by default") and `findings-flow.html`; the arch-fixtures names (`direction · unresolved · facade · dead-port · queue-table`) were placeholders.

## Decision

`.arch/rules`, schema version 0, the `arch init` template:

```toml
[[rule]]
subject = "domain"
must_not = "depend-on"
targets = ["driving", "driven"]
level = "block"

[[rule]]
subject = "driving"
must_not = "depend-on"
targets = ["externals"]
level = "block"

[[rule]]
subject = "domain"
must_not = "depend-on"
targets = ["externals"]
level = "block"

[lints]
god-module = "warn"
cycle = "warn"
leaky-port = "warn"
speculative-abstraction = "warn"
unresolved-dyn = "warn"
```

`.arch/allows`, schema version 0:

```toml
[[allow]]
site = "src/app/notify.rs::NotifyCustomer::deliver"
rule = "domain must not depend-on driven"
target = "src/adapters/email/mod.rs::LogNotifier"
by = "titouan"
reason = "log fallback while the mail provider is unreliable; remove when the retry policy lands"
```

| file · field | value |
|---|---|
| `rules` `[[rule]]` `subject` | a side (`driving` · `domain` · `driven` · `externals`) or an area name |
| `rules` `[[rule]]` `must_not` | `depend-on`, the only relation in V1 |
| `rules` `[[rule]]` `targets` | sides or area names |
| `rules` `[[rule]]` `level` | `block` · `warn` |
| `rules` `[lints]` `<name>` | `block` · `warn` · `off`; a lint not listed is off; the V1 lints are `god-module` · `cycle` · `leaky-port` · `speculative-abstraction` · `unresolved-dyn` |
| `allows` `[[allow]]` `site` | `<file>::<path of the item in its module>`, an impl's items under their type; or the item id ([ADR 0031](0031-items-and-item-ids.md)); or the link witness's `<file>:<line>` |
| `allows` `[[allow]]` `rule` | `<subject> must not depend-on <one target>`, worded as in `rules`, or a lint name |
| `allows` `[[allow]]` `target` | optional: the link's far end, in the form of `site` or the id of an external, port or table; it covers what sits under it (`…::LogNotifier` covers `…::LogNotifier::send`); absent, the allow covers the whole site |
| `allows` `[[allow]]` `by` · `reason` | the person who allowed it (agents cannot write this file, spec §7), and why |
| `allows` `[[allow]]` `at` | optional, when recorded |

- arch publishes `schemas/arch-rules.schema.json` and `schemas/arch-allows.schema.json` next to `schemas/facts.schema.json`, generated from the `arch-facts` reader types with the same drift test.

Why (tau-rs/arch#13): the smallsvc shape already parses in both repos and states each lint's level, which the pages show next to its count; and with the names the template writes, the screens and the engine call each check the same thing.

## Consequences

- tau-rs/arch (#75): `Rules::v1_template()` writes `"warn"` instead of `true`, the reader stops accepting `true`/`false`, and the two schemas are published. arch-fixtures: nothing left (arch-fixtures#12 renamed the lints).
- What each lint checks is not decided here: tau-rs/arch-design#123.
- [ADR 0006](0006-arch-init.md) and the **lint** and **allow** entries of `spec/vocabulary.md` point here.
- One honest consequence: an allow names its rule by wording, so rewording a rule, or renaming an area it names, detaches the allow and the finding comes back. It fails safe: a stale allow shows a finding, it never hides one.
- A second honest consequence: a site in file form names an impl's items by their type, so `src/x.rs::S::send` covers both an inherent `send` and a trait's `send` on `S` in that file. An allow that must cover one only uses the item id.
- A third: the `rule` of an allow names one target, so excusing a link to `driving` and one to `driven` under the same rule takes two allows.
- Rejected: rule ids in `rules` and `allows` (rewording keeps the allow, but each rule needs an id nobody reads); a one-line plain-text syntax (`domain !-> driving, driven`; a parser and error messages of its own); `name = true` for lints (cannot say `block`); the arch-fixtures names (the screens and the engine would print different names for the same check until the pages were rewritten).
