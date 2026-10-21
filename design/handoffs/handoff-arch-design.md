# handoff · `tau-rs/arch-design` · the record

Purpose: the one place where the product's decisions live, with a git history. Every other repo links to it; none depends on it at build time. Seed it first, because the other handoffs point at files that must exist here.

## What goes in

```
arch-design/
  README.md              what arch is in one page; the repo map; links to the other repos
  spec/
    arch-v1-spec.md      the V1 specification (attached; §13 holds the 2 Oct decisions)
    vocabulary.md        the terms (repo · unit · area · item · link · finding · witness · rule · plan ·
                         element · group · gate · session · sub-agent · scope · Focus · Lock · take over ·
                         remark · record · What's new · port · rail · node · tier · kind) with the C4 mapping
    keyboard.md          the keyboard map (⌘1 Map, ⌘B, ⌘J, ⌘⇧], ⌘K, ⌘P, ⌘↩, j k v r, Esc chain, f, arrows)
  flows/                 the HTML pages, one file each, self-contained (attached: shell.html, plan-shell.html,
                         session-shell.html, review-merge-shell.html, daily-shell.html, map-focus.html,
                         map-real-projects.html, v2-canvas.html; the older pages under flows/superseded/)
  adr/                   one file per decision, numbered, dated, in the format: context · decision · consequences
                         0001-store-sqlite … 0024-repositories (the 24 of spec §13), then one per later decision
  handoffs/              every handoff written so far (sett rebase, plugin spec, PoC results) and the per-repo ones
  roadmap.md             V1 · V2 · V3 as in spec §3, kept current
```

## Rules

- A decision is taken in the product chat and written here as an ADR before any repo implements it. An implementation session that needs a decision not in `adr/` asks the product chat; it does not decide alone and it does not leave the question in code comments.
- The spec is amended, not forked: when an ADR changes a section, the section is rewritten and the ADR is referenced.
- The flow pages are the visual spec. A page is superseded by moving it, never by editing it in place; a new page is a new file.
- Plain text, no build step, no generated content. Diagrams are the HTML pages or Mermaid in Markdown.

## Sync with the other repos

Weekly, or at every milestone of a repo: an implementation session reads `adr/` newer than its last sync and `roadmap.md`, applies what changed, and files its own findings (a surface the shell lacks, a fact the analyzer can't produce, a component sett refuses) as issues here labelled `from:<repo>`. The product chat triages those into ADRs. Each other repo's README states the ADR number it is synced to.

## Deliverable

The repo seeded with the attached files in the layout above, the 24 ADRs written from spec §13, README with the repo map, `roadmap.md`. Then it is maintained by the product chat and read by everyone.
