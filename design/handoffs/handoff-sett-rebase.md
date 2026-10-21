# sett · handoff · rebase the roadmap on the retained shell and the V1 spec

> Amended 2026-10-04 from arch-app FINDINGS F-10 (tau-rs/arch-design#40): `sett-findings-view` added (§3 item 4b, lane C).

Oct 2026. For the `tau-rs/sett` chat. Context: since sett's HANDOFF.md and ADR 0001 were written, the arch product chat (a) replaced the shell, (b) re-seated every flow on it, (c) settled the map decisions the PoC left open, and (d) froze a V1 spec. sett's lanes were planned against the old frame. This handoff lists what to keep, what to change, what to add, and the order, so sett ships what arch V1 consumes.

Inputs: `arch-v1-spec.md` (§4 shell, §5 map, §6 flows, §12 open items); the shell artifact https://claude.ai/artifact/KENuEgpvSjX4p7jRTGxebn; the four flows on the shell: plan https://claude.ai/artifact/5yqj554jto8B96bvepuAra · session https://claude.ai/artifact/Eu2VyYvcjsDziTCoB271Dc · review and merge https://claude.ai/artifact/1MxQnkAfsxh61hAQWkvkqU · daily https://claude.ai/artifact/Uq9zaFSn3WDbDZSXJ2XpF6; plugins https://claude.ai/artifact/JGtK8hmBmkakcvRkwqWkmy and https://claude.ai/artifact/CVJt77dw8uCTQgYvkmsNYF. These pages share one CSS and one set of helpers; read them as the reference rendering for the new components (match them, don't copy the CSS; DESIGN.md still wins on look).

## 1. Keep (already right)

Tokens (DTCG, light/dark, `tokens.rs`), DESIGN.md principles and rules 1–12, the motion budget, the glyph vocabulary, ADR 0001 (the map is DOM), lanes 2–6 of the map (node, port-row, rail, op-row, area, item, column, edge, link, hint-chip, ghost, panel, position, minimap, crumb, back, cue, code-page, contract-card, legend), the thread family, cards, split and gated buttons, tabbar/seg/overlay-toggles, editor hints, chip, pill, tag, frame, selector + menu, session-card. The a11y gate, the no-hex check, the generated `llms.txt` and skill.

## 2. Change

| where | from | to | why |
|---|---|---|---|
| DESIGN.md P-1 | as written | keep; the product chat adopted sett's wording (agent door first, "me first" swaps) | alignment |
| DESIGN.md rule 9 | pause · take over · stop; `✋` on the card only | keep; the product renamed its "step in" to **take over**; add **Focus** (make a session the scope) and **Lock** (pin a focus) as the two scope verbs, distinct from take over | vocabulary |
| `sett-session-card` | plan · lanes · files · git rows | add a **group** row kind (a lane with its gate state: done · running · gate · failed n/m · waiting), sub-agents folded under the group; the card's verbs bar reads pause · stop / resume · take over · stop; the composer below it becomes the hand-back note when an element is held | the waves policy and the gate states |
| `sett-selector` | branch states | the selector is the **scope selector**: main · `w1 · refund flow` (session colour) · `you · fix-pool-size 🔒` · `plan · refund flow` (amber); menu grouped Planning · Yours · Needs you · Running · In review · Done | the shell |
| `sett-frame` | idle · live · waiting · editing · collision · focus | add **planning** (amber, still); "focus" means the sel ring for the selection, not the plan | plan scope |
| `sett-chip` | kinds git/agent/finding/review/pipeline/tree | add kinds **gate** (running · failed n/m) and **detected** (changes detected · n files); every chip carries a verb; a done chip gains dismiss; (plugin kind is V2) | the bar |
| `sett-tabbar` | pinned Map, file tabs | file tab underlined in the session colour or sel for `you`; a Review tab (`Review · !44`) and a Plugins tab are center tabs | review, plugins |
| editor | hints | planned element = gutter glyph + hint pill at the site; agent change bars per line in the author's colour; `you` bars in sel; cross-repo symbol italic | rule 12 applied to the plan |
| map `layers` | leaves left, public API right | **public API left, leaves right**, so "uses" points left → right in both column rules; add column tints for layers (reuse driving/domain/driven tints by depth or add `surface.layer.*`) | direction fix |
| `sett-node` foot | open / enter acts | drop `▾ open`; double-click or ↩ is the only open | PoC decision |
| map levels | repo · areas · items | one board; repo › unit is open-in-place (trivial in V1); fold keeps the footprint (floor 0.7) | PoC decisions |
| `sett-rail` (map) | sections | add an **unresolved** section at the end (externals without an owner, unresolved links folded to one pill per item) | MAP-32 |

## 3. Add (new components, in arch's order of need)

1. **`sett-activity-rail`**: 56 px, glyph + horizontal label, items Sessions · Files · Findings; badges (asks-you, new-blocking); a 2 px bar in the session colour on the active label when a session is focused; never hides. States: active, badge, scoped.
2. **`sett-scope-line`**: the left pane's first row: dot, name, sub, lock; tints for main (neutral), you (sel), session (its colour), plan (amber). An indicator, never a control.
3. **`sett-sessions-view`** (or rows for it): grouped sections, session row (dot, name, state pill, Focus button on the selected row, chevron), group row, sub-agent row, file row with status letter and counts, Changes row, `+ new session · delegate`. Isolated mode (`‹ All sessions · N`).
4. **`sett-files-view`**: scope line, agent strip (collapsed path, unfolds), projection seg (Directory · Layers), tree rows with presence bar, status letter, writer label when focused.
4b. **`sett-findings-view`**: the left pane under Findings in the rail (arch-design#40): scope line, a count line (`n · m block`), group rows per rule (rule name, level glyph, count; blocking rules first, folds with a chevron), finding rows under their rule (level glyph, the link `from → to` or the item, `file:line`), only what the scope introduced. A row selects like a panel row: the Map goes to the item with the findings overlay, the inspector shows the fix card. Empty state is `sett-empty`. States: default, blocking, empty, scoped, selected row.
5. **`sett-panel`** (bottom): tabs Findings · Checks · Terminal · What's new with counts, closed strip of 30 px, tables for Findings (level · finding · rule · witness · origin) and Checks (check · where · result · when · output), terminal block, What's new lines.
6. **`sett-status-bar`**: counts and states only, each a link; right side Ln/Col and language, map freshness.
7. **`sett-inspector` layouts**: card + verbs bar + composer; checklist (glyph · fact · source link) with a gated button and a "how" block; result pane with merged · archived pills; plan delta; What's new doors; commit form (message, description, files with pick hunks, checks, then-radio, behind-main line); fix card; Ask thread (ran lines, answer with chips and witness tags, resolve rows for a judgement).
8. **`sett-changes-list`**: Magit-style stages (unstaged · staged · commits) with folders inside, status letters, viewed ✓, progress; the MR row; the Plan row.
9. **`sett-hunk`**: header (file:line · item · sub-agent · viewed · r · show on map), lines, a remark block with its two verbs.
10. **`sett-intent-bar`**: the plan's intention above the Map (input, counts).
11. **Plugins** (V2, not now): the Plugins tab, plugin row, inspector detail, the `+` door, origin tag, `not-evaluated` row state, the acceptance block. Reserve the names; build nothing.
12. **`sett-question`** and **`sett-deviation`** variants: the gate-failed question (four doors) and the denied-write deviation (check id, three typologies).

## 4. Order (lanes)

```
lane A  DESIGN.md amendments (§2 rows 1–3, 8, 9) · tokens: planning frame, chip kinds, layer tints, surface.unresolved
lane B  activity-rail · scope-line · status-bar · panel          (the shell can be composed)
lane C  sessions-view · files-view · findings-view · changes-list · intent-bar   (left pane and plan)
lane D  session-card groups · selector scope states · frame planning · chip kinds · tabbar session tabs
lane E  inspector layouts (checklist, result, delta, whatsnew, commit, fix, ask) · hunk
lane F  map amendments: layers direction and tints, node foot, unresolved section, footprint fold
lane G  (V2) plugins components
recipes  "plan · shaping" · "session · gate failed" · "review · glance" · "daily · edit by hand" composed from components, matching the four flow pages
```

B and C unblock arch's shell; D and E unblock the flows; F closes the PoC decisions; G is V2.

## 5. Stories and tests to add

Every new component: a story per state in both themes through the a11y gate. Recipes: the four above, each a composed screen. Tests: the scope is written in three places with the same words (scope line, selector, frame) from one source; the activity rail keeps badges when the pane is closed; a chip always has a verb; a planned element never renders as an inserted line.

## 6. Report back

Token count per file after lane A; the list of components built per lane with story counts; anything in the flow pages sett changed and why (as the ADR did for the PoC); anything the flow pages need that sett refuses (a modal, a toast, an animation outside the budget) as a finding for the product chat.

## 7. Sync with the other repos

- **`arch-design`**: the ADRs are the source of decisions; DESIGN.md wins on look, the ADRs win on behaviour and vocabulary. Read ADRs newer than the one in README's "synced to" line before each lane; file refusals (a modal, a toast, motion outside the budget) and gaps there (`from:sett`).
- **`arch-fixtures`**: stories load `fixtures-for-ui/`; request a fixture there before inventing data in a story.
- **`arch-app`**: the consumer; each lane's components are announced with the release; when the app files a missing component in `arch-design`, it lands in the next lane.
- **`arch`**: no dependency; a new row kind or state in `arch-views` arrives through `arch-design`.
