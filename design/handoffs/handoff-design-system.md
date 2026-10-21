# arch · handoff · design system

Purpose of this side-work: produce the design system that every arch screen will be built with, so that the V1 implementation (Claude Code) has tokens, components and states to reference instead of the mock CSS scattered across the flow pages. Output comes back to the product chat for verification before implementation.

## 1. What arch is (context you need)

arch is a desktop dev tool for Rust codebases (Eclipse Theia platform, web tech). It computes an architecture map from rust-analyzer facts and lets a person and coding agents work on a repo through that map: plan a change, delegate it to a session, follow it, review it, merge it. Vocabulary: **repo · unit · area · item · link · finding · witness · rule · plan · element · session · remark**. The map is the center of every screen.

Two principles that constrain visual design:

- **P-1** every action has a manual path and an agent path, both visible; manual is the default door. Visually: every decision surface shows both verbs, never a single "let the AI" button.
- **P-2** arch is a reader of the files, never their owner; nothing pops up, nothing blocks. Visually: hints and chips, never modal dialogs for state; one frame that changes colour instead of banners.

## 2. The shell (fixed, do not redesign)

One window, three columns, four bands:

- **Top bar**: brand, workspace name, branch selector (with a state label: planning · asks · paused · done), readiness marks (●●●), Ask ⌘K.
- **Strip**: the branch name and the **Actions chips**: each known problem as a chip with its links, ordered by what unblocks the merge (`behind main · 2 · Update myself · with Yokohama`).
- **Body**: left pane (files / changes / review views, the session card pinned at top when a session owns the branch), center (Map tab pinned at ⌘1, file tabs VS Code style), right pane (always "about the selection": Code | Reach, or a thread pane: framer, planner, session, fixer).
- **Status line**: findings, rules, uncommitted, map freshness.

Around the body, **the frame**: a 3-px border whose colour and motion are the only animation in the product: violet animated gradient = an agent is working on this branch; amber pulse = an agent waits for you; blue still = your uncommitted work; red still = collision; amber still = a focus (planning). The content never fades or pulses; only the border.

## 3. Reference artifacts (read these, they are the spec of what to style)

| page | what it fixes |
|---|---|
| base flow · https://claude.ai/artifact/RPXYVep3QJFZuZ68zwSkFY | shell, map, files, selection, Reach |
| git flow · https://claude.ai/artifact/DYLTmUXFxat7f3vE27jYZk | Changes view, stack, update dialog, actions, conflicts |
| findings flow · https://claude.ai/artifact/Qcst7B3a2PWsPzWbZSEggN | finding chip, fix card, allow, findings pane |
| ask flow · https://claude.ai/artifact/4hmAvh52j5hfGoWhkZ1fHy | question, queries ran, answer with witnesses, judgement with resolve rows |
| plan flow · https://claude.ai/artifact/TvLPb23m3CxwhLZooqdMxi | the focus (amber frame), intention bar, planner thread, split-button Accept |
| session flow · https://claude.ai/artifact/Tv9ST2JoKzRxDu5dLDCh7n | repo screen, session card, live editing, questions, deviations, session bar, step in |
| review flow · https://claude.ai/artifact/UrRzr9VVzXcb3MP87sTWyT | change list, hunks, remarks, checklist, verdict |
| merge flow · https://claude.ai/artifact/2oskEYshxjgwYkyH1kHmxj | gated button, pipeline block, description, merged result |
| sync flow · https://claude.ai/artifact/CGrQsXSw9ghaN8GmQkQMEJ | change detected, What's new pane, impact list |
| map focus · https://claude.ai/artifact/PTACFVEdRRHB4uYKW7dgTr | map anatomy, folding, overlays, invariants |
| real projects · https://claude.ai/artifact/PQoTSQ8JU6avRADKDRZjwq | map at scale, units, layers, families |
| V2 canvas · https://claude.ai/artifact/K8ur1qKoCYb3up5c1T8Cwa | zoom levels and tiers (V2, but the tokens must not preclude it) |

Onboarding and firsts pages exist but are out of V1; ignore them for tokens.

## 4. What to produce

### 4.1 Tokens
- **Colour**: paper, well, ink, ink2, mute, line, line2; semantic: sel (selection, blue), ok (green), bad (red), sug (amber, "suggested / planned / waiting"), yk (session colour, violet) plus a palette of 6 more session colours with two lighter shades each (session, sub-agent); dark theme for all of them. Sessions are identified by colour everywhere (frame, dot, card border, message border, map overlay), so the palette must stay distinguishable at 12 px.
- **Type**: a sans for UI (currently IBM Plex Sans), a mono for identifiers, code and map labels (IBM Plex Mono). Sizes: 10.5 / 11 / 11.5 / 12 / 12.5 / 13 / 14; map labels are one size only (9.5–10 px equivalent) and never scale.
- **Spacing and radius**: 4-px grid; radii 3 (items), 4 (chips), 6 (cards), 8–10 (nodes, panes).
- **Motion**: only the frame (gradient rotate ~6 s, pulse ~1.6 s), one pulse on a changed map item, transitions between zoom levels (V2). Nothing else moves.

### 4.2 Components (with every state)
1. **Chip** (Actions strip): kind (git · agent · finding · review · pipeline · working tree), one primary link, secondary links; states normal / blocking / waiting.
2. **Frame**: idle, live (per session colour), waiting, editing, collision, focus.
3. **Selector** (branch): dot states, state label pill.
4. **Session card** (left pane): header, element rows with state glyphs (✓ · ● · ⏸ · ✋ · ≠ · ! · ·), foot link.
5. **Thread pane** (framer / planner / session / fixer): header with colour top border, messages (me · agent · sub-agent shades), tool call block, "changed · what / no change" line, question block with options, deviation card with typologies, **fixed verb bar** above the composer, composer.
6. **Map primitives**: area box (normal, suggested, editing), item (normal, port, external, selected, lit, faded, planned, session-touched, sub-agent, finding), link (normal, dashed unresolved, right-to-left smell, planned, session, finding pill), area header, chip (folded area / unit), socket (V2), selection box with handles and the action pill, unplaced tray, whiteboard toolbar with tooltips, zoom control, minimap, legend line.
7. **Editor inlays**: witness, planned element, "you", session name, finding ⚠, cross-repo.
8. **Cards**: fix card, plan delta card, impact list, merge checklist (line = glyph · fact · source link), pipeline block, result pane, What's new list.
9. **Split button** (Accept · delegate ▾ | Save plan), gated primary button with its reason line.
10. **Tabs**: center tab bar (pinned Map, file tabs, right-side controls), left pane view switch (Files | Changes | Review), map level switch (repo · areas · items) and overlay toggles.
11. **Funnel** (onboarding, out of V1 but tokens should allow it).

### 4.3 Rules to encode
- Verbs live in a pane's fixed bar or in a chip; never in a modal.
- A blocked state is a pill wherever its subject is named.
- Manual door first in every pair (`fix myself · with Yokohama`).
- Text in chips and cards uses `·` as the separator, lowercase labels, mono for identifiers.
- Dark mode is required from day one (developers).

### 4.4 Deliverables
- A tokens file (CSS variables, light and dark) and a components page (HTML/CSS or Figma, your choice; if Figma, also export the tokens).
- One composed screen per reference page rebuilt with the components, to prove coverage (the session "live" state and the map "edit at scale" state at minimum).
- A short written note of anything in the reference pages you changed and why.

## 5. Constraints
- Theia hosts the UI; components must be plain CSS/DOM-friendly (no framework lock-in); the map itself will be canvas/WebGL, so map tokens must be expressible as plain values, not CSS-only.
- Density is high: developers, 13" laptops; nothing larger than 14 px except headings in panes.
- No icons library dependency for V1; glyphs from Unicode are acceptable where the mocks use them, otherwise a small custom SVG set.
