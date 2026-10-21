# handoff · `tau-rs/arch-app` · the Theia product

TypeScript. The desktop app: the shell, the map, the views, the editor decorations, the API client. It computes nothing about architecture: it subscribes to views from `arch` and sends intents. It owns no styling: every visual element comes from **`@tau-rs/sett`**. Read `arch-design/spec/arch-v1-spec.md` (§4 shell, §5 map, §6 flows, §13), the shell page and the four flow pages, and sett's `DESIGN.md` (normative on look) first.

## 1. Layout

```
arch-app/
  packages/
    arch-client      generated from arch-api's JSON schema: typed methods, event subscriptions; regenerated on
                     every schema version; the only coupling with the engine
    arch-shell       the frame: one bar (scope selector, chips, Ask), the rail (Sessions · Files · Findings),
                     left pane with scope line, center (Map pinned, file tabs, intent bar), inspector, bottom
                     panel (Findings · Checks · Terminal · What's new), status bar, the frame border states;
                     scope and Focus/Lock logic
    arch-map         the map: board (V1: one unit), unit sheet (columns, areas, items, rails with ports, lines,
                     bundles, levels plugs · areas · items, light · pin · open-by-hand), overlays, the whiteboard
                     tools and Unplaced tray; built from sett's map elements; DOM per sett ADR 0001
    arch-views       the subscribers: Sessions view, Files view (Directory · Layers), Findings view, Changes list,
                     checklist, session card, thread panes (planner, agent), cards (fix, delta, impact, result,
                     What's new, commit), Ask thread, Settings tab
    arch-editor      Theia editor integration: gutter marks (agent change bars in author colour, planned
                     element glyph), hints after the line, underlines (finding wavy, witness span); no inserted lines
    arch-terminal    Theia terminal per worktree, named by the scope
  applications/
    electron         the desktop build; spawns `arch serve`; keychain through Theia's secret storage
  design/
    theme.json       semantic overrides only (never raw colours; sett tokens are the source)
```

## 2. What the app does and does not do

- Renders views and sends intents: focus, lock, open, answer, decide (deviation doors), take over, hand back, accept, save plan, discard, commit, create PR, approve, merge, allow, place, mark seen. Each intent is one API call; the UI never mutates state locally beyond optimistic hints.
- Never computes positions, findings, or plan state; never reads the repo's files except through the API or Theia's own editor.
- The scope is written once (the selector, the scope line, the frame colour) from one subscription.
- No modal dialogs anywhere; the Update dialog from the git flow is a card in the inspector.
- Motion only as sett's budget allows.

## 3. sett

sett is the component source, not a theme. The app imports `@tau-rs/sett` (tokens CSS, element bundle, the generated skill for agents working on this repo) and composes `sett-*` elements; it never redefines one. When a flow needs a component sett lacks, the app does not improvise it: it files the need in `arch-design` (`from:app`) and uses the closest sett element meanwhile. The app's Storybook composes sett's stories with `arch-fixtures/fixtures-for-ui`. Sett's lanes A–F (see `handoff-sett-rebase.md`) are what the app waits on; the app's own milestones are ordered to consume them as they land.

## 4. Milestones

1. **Shell on fixtures**: `arch-client` generated; the frame renders from `fixtures-for-ui` with no engine; scope, Focus, Lock, rail, panel, status bar. (needs sett lanes A–B)
2. **Map on fixtures**: the unit sheet on smallsvc and zero2prod golden views; levels, light, pin, overlays; MAP invariants from `arch-fixtures` checked in the browser. (needs sett map lanes)
3. **Daily flow live**: against `arch serve`: map and selection, read code, Ask, edit by hand → `you` session, finding, What's new, commit. (needs sett lanes C–E)
4. **Plan and session**: planning row, intention bar, planner thread, Accept · delegate | Save plan, Sessions view with groups, session card, asks, deviation, gate, gate failed, take over.
5. **Review and merge**: glance, detail, remark → plan delta, verdict, merge pane, merged · archived.
6. **Electron**: spawns the engine, keychain, settings, packaging for macOS and Linux.

## 5. Tests

Storybook on fixtures for every view state; Playwright on the composed shell for the four flows against the engine on smallsvc; a schema-drift test (client regenerated == committed client); an a11y pass in both themes (sett's gate extended to compositions).

## 6. Sync with the other repos

- **`arch`**: regenerate `arch-client` on every published schema version; a method the app needs that the schema lacks is filed in `arch-design` (`from:app`), never worked around.
- **`sett`**: track its releases; after each lane lands, replace any placeholder with the real element; file refusals and gaps in `arch-design`.
- **`arch-fixtures`**: load `fixtures-for-ui`; when a flow page gains a state, request the fixture there before building the state.
- **`arch-design`**: read new ADRs before each milestone; the flow pages are the acceptance reference; the README states the ADR synced to.

## 7. Definition of done for V1

The four flow pages reproduced live on smallsvc and zero2prod through `arch serve`, with sett components only, budgets met in the browser, the V1 scenario from `arch-fixtures/scenarios/v1-done.md` passing end to end in Electron.
