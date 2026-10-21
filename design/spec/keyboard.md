# arch · keyboard map

Indexed from `arch-v1-spec.md` and the flow pages. The spec wins where a page differs (spec, preamble); differences are listed at the end for the product chat.

## Shell (spec §4)

| key | does |
|---|---|
| ⌘1 | the Map tab (pinned first in the center) |
| ⌘B | folds the left pane; not while reading |
| ⌘J | toggles the bottom panel (Findings · Checks · Terminal · What's new); closed is a 30 px strip with counts |
| ⌘⇧] | folds the inspector to a 28 px handle |
| ⌘K | Ask |
| ↩ | on a session row or card: Focus (makes the session the scope) |
| Esc | unfocuses (back to all sessions) |

## Map (spec §5)

| key | does |
|---|---|
| ⌘P | finds items, files, areas, rules |
| ↩ | selects and opens where it is (code opens as a tab only by double-click or ↩) |
| ⌘↩ | opens the file as a tab from the ⌘P result |
| f | fit |
| arrows | walk the links from the selection |
| Esc | the chain: tab → unit → focus → level |
| double-click | opens code as a tab (MAP-5) |
| V · H · A · L | whiteboard toolbar: select · pan · area · lasso |

## Plan (spec §6)

| key | does |
|---|---|
| ↩ | in the intention bar: drafts the plan |

## Review (spec §6)

| key | does |
|---|---|
| j · k | next · previous hunk |
| v | marks the file viewed (✓ in the Changes list, the checklist count moves) |
| r | opens a remark on the hunk |

## Daily (spec §6)

| key | does |
|---|---|
| ⌘↩ | commit, one click from the `you` chip |

## Seen in pages, not in the spec

For the product chat to confirm or drop; not decided here.

| key | page | reads as |
|---|---|---|
| ⌘W | map-focus | close the tab |
| ⌘0 | map-focus | fold all areas |
| ⌘⇧E · ⌘⇧A · ⌘⇧M | shell | rail views (Files explorer · Sessions · ?) |
| ⌘⇧F | shell, flows | filter in the Files view |
| ⇧1 · ⇧2 | map-focus | fit the current level · fit the selection's area (the spec says `f` fit) |
