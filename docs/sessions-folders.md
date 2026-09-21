# Sessions folders

Status: implemented 2026-09-17 (Controller `"folders"` sync state, Mobile
drawer, Desktop rail). "Implementation notes" below records where the shipped
behaviour deviates from the first design.
Scope: the Sessions sidebar on Mobile (spatial drawer) and Desktop (rail),
the `"folders"` synced state, and its Controller persistence.
Non-goal: search/filter of sessions, multi-selection, tags or colors.

## Why folders, and why not a copy of Obsidian's explorer

Cowboy's Sessions list is one flat, user-ordered list. A single developer
running many agents accumulates dozens of sessions across projects (`cowboy`,
`garden`, `stormbird`, …) and machines. Obsidian's file explorer is the
existence proof for a folder tree that works with a finger and with a
keyboard, so its interaction grammar (collapse/expand, inline rename,
context menu, "Move to…" picker, ancestor reveal, keyboard tree motions) is
reused where it fits. Three things make Cowboy sessions unlike notes and
drive every deviation below:

1. **A session already knows its project.** `workspace_name`/`workspace_id`
   or the Columbus project path (`sessionProjectLabel`) is a stable identity,
   the same repository on Hawk and on the Mac. Filing every new session by hand,
   as Obsidian requires for notes, would be friction without benefit; a folder
   may therefore be *bound to a project* so new sessions land there
   automatically.
2. **Sessions are live agents.** A folder must surface the state of what it
   contains (a busy or attention-needing session inside a collapsed folder),
   and deleting a folder must never delete sessions.
3. **Two products share one list component.** Both surfaces render the same
   session direction; Mobile adds paint-only chrome inside the swipe
   compositor, while Desktop is keyboard-first with a checked-in binding
   policy. The tree is one data structure with two presentations, never two
   lists.

## Model

One new synced state, `"folders"`, exactly parallel to `"order"`: optimistic
on the client, arbitrated and persisted by the Controller, replayed on
reconnect, projected per principal.

```jsonc
{
  "folders": [
    { "id": "f-…", "name": "Cowboy", "parent": null, "position": 0, "project": "cowboy" },
    { "id": "f-…", "name": "IME",    "parent": "f-…", "position": 0, "project": null }
  ],
  "placement": { "<session-id>": "f-…" }
}
```

- A folder is user-owned, named, nested (`parent`, `null` = root), ordered
  among its siblings (`position`), and optionally **bound to a project**
  (`project`, unique per user).
- `placement` holds only *explicit* moves; the empty string is the explicit
  top level. The **effective folder** of a session is derived on the client:
  the top level when `placement[id]` is `""`; `placement[id]` if that folder
  exists; otherwise the folder bound to `sessionProjectLabel(session)`;
  otherwise the root. Renaming, nesting or moving a bound folder therefore
  never orphans sessions, a new session of a bound project appears in its
  folder with no write, and "Move to → Top level" still beats the binding.
- Session order stays the existing global `"order"` array. A folder shows its
  sessions in that order filtered to its members; reordering inside a folder
  submits only that folder's ids, which `merge_session_order` permutes in
  place (server semantics unchanged).
- Root shows folders first (by `position`), then unfiled sessions. Every
  container follows one shared session direction on both surfaces: the synced
  `"order"` array reversed, newest first. Desktop and the Mobile drawer must
  never disagree about where a session sits; `Alt/Option+1…0` numbers that same
  displayed order, so the keycaps read 1…0 down the Desktop rail.
- Collapsed state is per device (`persisted("cowboy:session-folders:collapsed")`),
  like Obsidian's localStorage folds; it is presentation, not shared data.
  Folders start expanded. Opening a session (row tap, `Alt/Option+1…0`, push,
  Desktop region entry) reveals it by expanding its ancestors.
- Deleting a folder moves its subfolders and sessions to its parent (root for
  a top-level folder). Nothing else is destructive; sessions are deleted only
  through their own existing Delete flow.

### Mutations (client mutator → arbiter)

| name | args | rule |
|---|---|---|
| `create` | `{id, name, parent, project?}` | client-minted `f-<uuid>`; name trimmed, non-empty, ≤ 80 chars; parent must exist or be null; `project` unique per owner |
| `rename` | `{id, name}` | same name rules |
| `move` | `{id, parent}` | rejects cycles (folder into itself or a descendant); appends last among new siblings |
| `reorder` | `{parent, order: [id]}` | sibling permutation, same semantics as session order |
| `bind` | `{id, project \| null}` | unique per owner; `null` unbinds |
| `place` | `{session_ids: [id], folder \| null}` | explicit placement; `null` = explicit top level (stored as `""`); unknown session ids are dropped by projection |
| `remove` | `{id}` | children and explicit placements move to the parent |

The arbiter validates before consuming the mutation id, dedupes retries, applies
to typed state, persists, and echoes `sync_patch`. Projection strips
placements of sessions the principal cannot see and folders owned by another
user; viewers cannot mutate folders (same predicate as reordering).

## Controller

- Migrations `0050_session_folders.sql` (PostgreSQL) and
  `sqlite/0024_session_folders.sql`: table `session_folders(id, owner_user_id,
  name, parent_id, project, position, created_at_ms, updated_at_ms)` and
  `sessions.folder_id` (nullable, no FK so a lost folder degrades to root).
- `Hub` holds `folders` and `placement` beside `order`, restores them with
  `load_all`, and serves `sync_value("folders")` from that typed truth.
  `sync_resync` always seeds `"folders"` with title and order so a stale
  IndexedDB overlay cannot win.
- `StoreWrite::ReplaceSessionFolders` writes the whole owner-scoped folder set
  in one transaction (the set is small); `StoreWrite::UpdateSessionPlacement`
  updates `sessions.folder_id` for a batch. Both storages implement both.

## Client store

`registerSync("folders", {kind: "service", state: "folders"}, folderMutators,
EMPTY)` beside `titleSync`/`orderSync`; `ProductSyncScope` gains
`service:folders`. `State.sessionFolders` exposes the raw value; a pure
`sessionTree.ts` builds display rows from `(sessions, folders, placement,
collapsed, direction)`: folder rows carry depth, expanded flag, direct and
nested counts, and an aggregated status (`busy` › `interrupted`/`crashed` ›
`running` › idle) computed from every descendant session, so a collapsed
folder still shows that an agent inside needs attention.

## Mobile

- Folder row: disclosure glyph (icon swap, never a rotating `transform`),
  name, aggregated `StatusDot`, count. Tap toggles collapse; the row is
  paint-only chrome inside the peek (mobile-spatial-presentation §2.1).
- Folder kebab (`ObsidianSheet` compact card, matching Rename/Delete):
  New session here (opens New Session with the bound project preselected),
  New folder inside, Rename, Move to…, Bind project…, Delete folder (confirm
  names the destination of its contents).
- Session kebab gains **Move to folder…**: a picker sheet listing Root and every
  folder as an indented tree, with the current folder marked and a trailing
  "New folder…" row; a search field appears above eight folders (Obsidian's
  Move-to modal, without its path syntax).
- Drawer footer "+" is unchanged; the New Session sheet gains an optional
  Folder row defaulting to the folder bound to the chosen workspace, else the
  folder of the current session, else Root.
- The grip still reorders. Dropping a session lands it in the container of
  the row above the drop position; directly below a folder header (expanded
  or collapsed) files it into that folder, so "drop onto the folder" works
  without a hover timer. Obsidian mobile has no drag-into-folder either, so
  "Move to…" remains the primary path.
- The footer's leading island gains **New folder** beside New session. The
  name prompt offers **By project (N)** while unbound project labels exist:
  one tap creates one bound folder per project that has none yet, writes no
  placements, and every current and future session of those projects files
  itself.

## Desktop

Rows (folders and sessions) are `data-desktop-item`s of the `sessions.list`
region, so the existing cursor, `j/k`, `gg/G` and `Alt/Option+1…0` slots keep
working. Slots number sessions by the flat global order, independent of
folders and collapse, so a slot never changes meaning when a folder folds.

Region bindings (bare keys are region-scoped; none is global):

| key | folder row | session row |
|---|---|---|
| `l` / `Enter` | expand (`Enter` toggles) | open, focus Prompt (unchanged) |
| `h` | collapse; if collapsed, focus parent; at root, collapse all | focus parent folder |
| `i` | Rename prompt | Rename dialog |
| `s` | folder actions modal | session actions modal (the former `h`) |
| `m` | Move folder to… | Move session to folder… |
| `n` | New folder inside | New folder beside |
| `o` | Order mode among sibling folders | Order mode inside the container (unchanged) |

`h` moves from "session Settings" to the tree's left motion because a tree
without `h/l` is not a Vim tree; the actions modal keeps every former action
(and gains `M` Move to folder…), and `s` is its mnemonic. The status line
lists the region's live map. Command Palette entries: New Session Folder,
Move Session to Folder…, Organize Sessions by Project, Collapse/Expand All
Session Folders, Reveal Current Session. No workspace-prefix continuation is
added; `Cmd/Alt+K N` still creates a session. A folder button beside New
session opens the same folder-wide actions with the pointer.

Pointer: dropping a dragged session directly below a folder header files it
into that folder (see Mobile); the same rule drives Order mode, so `j/k`
across a header moves a session in or out. The folder actions modal offers
the Mobile actions with lettered slots (`N` new session here, `F` new folder
inside, `R` rename, `M` move, `B` bind project, `X` delete).

## Implementation notes

- No 750 ms hover auto-expand: the bespoke sortable measures row slots at
  pickup, so rows must not appear mid-drag. "Below the header files into the
  folder" replaces it and also covers collapsed folders.
- Folder rename uses the shared name prompt (`FolderNameShell`) rather than
  an inline editor; it is mounted inside the opening tap like session rename
  so iOS raises the keyboard.
- The New Session sheet has no Folder row; "New session here" in a folder's
  menu files the created session, and project binding covers the rest.
- A rejected folder mutation never consumes its sync id (the arbiter dry-runs
  on a copy first), so a corrected retry under the same id still applies.

## Not in v1

Multi-selection and bulk move, a search/filter box, folder icons or colors,
machine-based grouping, and drag-into-folder on Mobile beyond the inferred
placement above.

## Verification

- Rust: mutator validation (cycles, uniqueness, name rules), projection,
  persistence round trip on both storages, resync seeding.
- Web: `sessionTree` derivation (direction, counts, status, effective folder,
  reveal), placement inference from a drop index, `productSyncDatabase` scope,
  shortcut audits (`chromeShortcutPolicy`, `macShortcutPolicy`,
  `shortcutRegistrationPolicy`, `desktopShortcutDesign`), source contracts
  around `SessionList`.
- Chrome bridge: Desktop keyboard map, drag-into-folder, inline rename.
- iOS Simulator: drawer swipe stays 1:1 with folder rows present, kebab and
  Move-to sheets keep the keyboard rules, no new compositor descendants.
