# Desktop Code Review

**Status: design proposal r3 (2026-11-08); D0–D4 implemented (D3 with a Web-side projection; see the phase notes for what remains).** r3 records the
owner's answers to the r2 open questions. r2 folds in a
Codex Astra Max design review of r1 (managed call
`call-53884a467577cfb02c34574f`: verdict *needs-attention*, five high and ten
medium findings, none rejected; see [Review record](#review-record)). Normative
contracts it builds on: [Desktop focus and keyboard](../web/src/desktop/FOCUS.md),
[Desktop efficiency](desktop-efficiency-redesign.md),
[core UX principles](ux-principles.md), the Code Review data plane in
[13-code-review](architecture/13-code-review.md), the capability engine in
[19-code-review-engine](architecture/19-code-review-engine.md) and agent code
intelligence in [20](architecture/20-agent-code-intelligence.md). Where this
proposal changes a sentence of FOCUS.md or 13, that document is updated in the
same change that implements the new behaviour (D0 for the keyboard contract).

## Problem

Desktop has Review today, but only as a guest: `␣V` mounts the phone's
`ReviewApp` inside the Conversation pane (`web/src/desktop/DesktopWorkspace.tsx`),
single column, bottom workbench, touch targets, drawer shell and all.

- **Width is wasted.** A diff and the list of changed files compete for one
  column while the window has room for three. Desktop's rule is the opposite:
  controls expand from the actual container width.
- **No keyboard grammar inside Review.** The six Desktop primitives (modes,
  motions, item verbs, leader, labels, direct chords) stop at the pane border.
  A Vim user cannot `]c` to the next hunk, `/` inside the diff, `gd` a symbol
  or `Enter` a selection into the Prompt.
- **Device state leaks across devices.** `ReviewApp` consumes the synced
  `mobile-review:<sid>` state wholesale, so Desktop's open file tabs and
  navigator mode follow the phone and vice versa, although 09-frontend already
  states Desktop should not consume that state.
- **Structure.** `ReviewApp.tsx` (2,407 lines), `DocumentView.tsx` (1,687) and
  `CodeViewer.tsx` (1,058) are ratchet offenders. Growing Desktop features into
  them is forbidden by `just structure-check`, and `surface === "desktop"`
  branches are exactly the debt the Desktop redesign removes.
- **Missing review tools.** No side-by-side diff, no hover or definition from
  the keyboard, no outline/hunk/problem list beside the code, and AI review
  findings (`review.start`) are read in the Calls dock, not on the lines they
  are about, although every finding carries `file`, `line_start` and
  `line_end` (`components/wire/src/managed_calls/review.rs`).

## Goals and non-goals

Goals, in the order of the UX principles (efficiency first):

1. The review loop — open the next change, read it, send a precise remark to
   the agent, mark it reviewed — is keyboard-complete and never leaves the
   session page.
2. A user with basic Vim knowledge is productive without Cowboy-specific
   memory: the document is a read-only Vim buffer, the lists speak Cowboy's
   list grammar, everything else is `␣V` + one key, drawn on the controls.
3. Review uses the whole work column and adapts its inner columns to the
   container width, never to a viewport breakpoint.
4. MUI semantics and theme tokens for every control that has an MUI
   interaction model; custom primitives only where MUI has none (the editor,
   splitters, keycaps, the status line).
5. Desktop and Mobile stay separate products sharing protocol, API client,
   models and hooks; neither grows responsive branches for the other.
6. Everything drawn on a line is bound to an identity the server vouches for:
   no old-side text, finding or annotation is reconstructed from a source that
   is not bound to the displayed revision.

Non-goals: editing files, terminals, writing comments to GitHub, a second
shortcut grammar, Monaco (see "Editor engine"), a Desktop reading mode of its
own (folding Sessions and Prompt is that mode), and any change to the Code
data plane's bounded reading model (paged diffs, windowed files, revision
fences).

## Target workspace

```text
┌ tab strip: Sessions and Drafts open on this device ─────────────────────────────┐
├ Sessions ┬ top bar: session · agent · model · effort · context · follow · stop ─┤
│          ├ Prompt ───┬ [Conversation ␣C] [Review ␣V] ───────────────────────────┤
│ tree     │ composer  │ Navigator        │ Document (file tabs)      │ Inspector  │
│          │           │ Changes Files    │ diff: unified | split     │ Outline    │
│          │ plan      │ History Worktrees│ source (read-only Vim)    │ Hunks      │
│          │ queue     │ ─────────────    │                           │ Problems   │
│          │ drafts    │ comparison       │                           │ Details    │
│          │           │ unread · fetch   │ ───────────────────────── │            │
│          │ send      │ / filter  Files|Text │ Ln 42 · hunk 2/5 · Send ⏎ Explain ␣VE │
│          │           ├ approval dock: the blocked agent's question, either view ─┤
├──────────┴───────────┴─────────────────────────────────────────────────────────┤
│ NORMAL  REVIEW·Document  main ↔ Ready to merge  3/51 reviewed  ]c Hunk  ␣V More │
└───────────────────────────────────────────────────────────────────────────────┘
```

The Sessions pane, the tab strip, the top bar, the Prompt and the status line
are unchanged. The work column (today: Conversation) gains a second **view**,
Review. Both views are mounted; switching never loses the transcript's reading
position or Review's cursor, folds and tabs.

The approval dock moves out of the transcript container to the work pane
itself, below whichever view is shown, so a permission, Provider question or
call approval that arrives while Review is open is answered where it was
before: at the pane's full width, outside any scroller, with the full subject
(FOCUS.md "Approvals"). The dock keeps claiming the Session, so the Composer
draws the choices only while the work pane is folded, exactly as today.

### Inner width policy

Review measures its own container (`@container`), never the viewport:

| Review width | Layout |
| --- | --- |
| ≥ 1320 px | Navigator (persisted, default 280 px) · Document · Inspector (persisted, default 300 px) |
| 900–1319 px | Navigator · Document; the Inspector is folded. Restoring one side panel folds the other, so one side panel is open at a time |
| < 900 px | Compact: both side panels fold to 36 px rails; the Document has the width. `␣VC/F/H/W` open the Navigator as a temporary inset list (MUI `Drawer` anchored inside the pane, an exclusive region) |

In the inset, the list keeps first ownership of its keys: `H` collapses a
tree node, moves to the parent or leaves a commit, as in the wide layout, and
closes the inset only at the navigation root (a top-level row with nothing to
collapse, or the change list itself); `Esc` leaves the filter field first and
closes the inset next; opening a file closes it. The Navigator is the same
mounted owner at every width, so a resize across a tier keeps its cursor,
folds and filter.

A laptop at 1440 px with Sessions and Prompt open gives Review about 750 px,
so Compact is the common first sight. Folding the Prompt (`␣W]`) is the
intended reading layout; Review never folds another pane on its own.

Inner splitters join Resize mode (`␣WR`, `Tab` cycles bars). Side-panel folds
are `␣V[` (Navigator) and `␣V]` (Inspector), mirroring `␣W[`/`␣W\`.

## Entering and leaving Review

Review is a persistent view of the work pane, like the History/Explore
projection, not a transient layer. Consequences:

- **`␣V`** opens Review from any focus, Composer Insert included (`Cmd/Alt+K V`).
  It also runs from the pane header's view tabs (`Conversation | Review`, an
  MUI `Tabs` with the `␣C`/`␣V` slots) and the Command Palette.
- **Return**: `␣C` (focus Conversation restores the Conversation view), the
  header tab, or the palette's *Return to the Conversation*.
- **`Esc` never leaves Review.** It unwinds Visual mode, a pending chord, the
  hover popover, the filter field or an inset list, and then does nothing, as
  in a Vim buffer. `Esc` is pressed constantly while reading code (leaving
  Visual, closing hover, cancelling `/`), so a product-mode exit on it would
  be an easy slip; leaving is always a deliberate `␣C` (owner decision). The
  earlier Desktop behaviour where Review's Agent action
  and Send to Agent return to the Conversation is dropped: the Prompt is
  already beside the code.
- **Agent-driven reveal** (`open {path, reveal}` on the synced state, used by
  agents and transcript file links) switches the view to Review and opens the
  file, but only as live navigation: hydration and session switches never
  steal focus (`useReviewOpenRequest.ts`; see "Reveal" below).
- The view is per Session and per device (`cowboy:desktop-work-view:<sid>`),
  so returning to a Session returns to where its review stopped.

### The `␣V` group

`␣V` becomes the Review group. FOCUS.md's group rule (implemented by
`desktopLeaderGroupAvailable`) opens a group whenever any of its commands can
run and otherwise leaves the key its root meaning. Every grouped command
therefore declares `when: reviewShown`, and the root command `review.open`
(the former `conversation.toggleReview`, whose hints, slots and tests
migrate) is the fallback: with Review hidden, `␣V` opens Review at once, as
today; with Review shown, `␣V` opens the group.

| Key | Meaning |
| --- | --- |
| `␣VV` | Focus the Document, as `␣SS`/`␣TT` focus their surfaces (`review.focus`); return is `␣C` |
| `␣VC` `␣VF` `␣VH` `␣VW` | Navigator: Changes, Files, History, Worktrees — switch the list and focus it |
| `␣VA` | Compare against: Uncommitted, Ready to merge, Not pushed, This session, other branches, Fetch this branch |
| `␣VU` | Unread filter on the change list |
| `␣VD` | Diff layout: unified ⇄ split |
| `␣VM` | Mark reviewed / unreviewed: the focused Navigator row's file, else the open file |
| `␣VS` | Send to the Prompt and stay in Review: the focused Navigator row's path, the focused Problem, else the Document selection or cursor line (collect several, then write once) |
| `␣VE` | Explain the selection: a fork of the agent answers in a new tab |
| `␣VR` | Review the selection or open file with an AI reviewer (the fork starts a managed call under the contract below) |
| `␣VK` | Checks: run a declared project check (`check@1`); a picker when several are declared; enabled only for the active Session's own worktree |
| `␣VT` | Text search across the worktree (`search@1`): the Navigator filter in Text mode |
| `␣VO` | Outline: focus the Inspector's Outline section (`gO` from the Document) |
| `␣VX` | Context: project checkout, linked sessions, bind a worktree, Remote PR |
| `␣VB` + `D/O/R/L/A/P/K/U/</>` | File tabs group, the `␣B` grammar one level down: close this, others, right, left, all; pin, keep, reopen; move |
| `␣V[` `␣V]` | Fold Navigator / Inspector |
| `␣V,` | Review settings: code font size, wrap, whitespace, context lines, hover on dwell |

Every control in the Review pane header, Navigator header and Document footer
carries its slot; the which-key panel lists the group under "Here" with the
Document's own keys first. Root `V` stays taken; the free root letters remain
`I X`.

## Keyboard grammar inside Review

Review adds no primitive. Its three regions are the Document (a Vim buffer),
the Navigator and the Inspector (Cowboy lists). `Ctrl+H/J/K/L` move between
them and to the Prompt and Sessions as between any regions; `␣WW` cycles them.
No list row carries a bare product letter: Mark and Send are leader actions
with the focused item as their target, so the shared item verbs (`M` move,
`S` settings) keep their meaning everywhere.

### Document: a read-only Vim buffer

The Document is CodeMirror 6 with `@replit/codemirror-vim`, the same engine,
vimrc subset (`vim/vimConfig.ts`) and `cmTheme` as the Composer, with
`EditorState.readOnly`. Every Vim motion, search, mark, count, register and
fold key therefore works unchanged, and the user's `map`/`noremap` lines apply.
Insert-mode entry (`i a o I A c s …`) is refused and the status line shows
Vim's `W10` wording, "Read-only". The buffer is one file: a unified or split
diff, or a source window.

codemirror-vim's keymap is global and the user's mappings are applied to it
globally (`vim/imeAutoInsertVim.ts`), while the Composer and a Draft editor
may be mounted beside Review. Review's additions are therefore **not**
`Vim.map`/`defineAction` registrations. They are a Review-editor-scoped CM6
keymap extension that runs only while that editor's Vim state is Normal or
Visual with no pending operator, count or user-mapped prefix, yields to any
user mapping of the same keys (the vimrc wins), and leaves the other editors
untouched. Each is also a Desktop command for which-key, the palette and the
status line.

| Key | Action |
| --- | --- |
| `]c` / `[c` | Next / previous hunk (Vim diff mode) |
| `]f` / `[f` | Next / previous changed file over the complete queue, whatever the Unread filter shows (13-code-review: the filter never changes navigation); the ends stop and the status line says "Last change" |
| `]u` / `[u` | Next / previous unreviewed file; a distinct command, so marking the current file never changes where `]f` goes |
| `]d` / `[d` | Next / previous problem in this file: language diagnostics, check findings and AI findings in one ordered set |
| `gt` / `gT` / `{n}gt` | Next / previous / nth file tab; `␣VB` holds close, keep, pin, reopen and move |
| `zo zc za zR zM` | Folds: collapsed unchanged regions in a diff, language folds in source |
| `K` | Hover (type, documentation) in an MUI `Popover` at the cursor; `K` or `Esc` closes, `j/k` scroll it |
| `gd` `gD` `gy` `gI` | Definition, declaration, type definition, implementation (LazyVim's LSP set): opens the target in the preview file tab and pushes the jump list |
| `gr` | References: the Inspector lists them, `Enter` jumps |
| `gO` | Outline in the Inspector (Vim's `gO`) |
| `Ctrl-O` / `Ctrl-I` | Jump list back / forward, across files |
| `V` `v` | Visual selection; a click on a line number selects the line and a Shift-click extends it, as on Mobile |
| `Enter` | Send the selection (or cursor line) to the Prompt: appends the location plus the text to the Composer and focuses it in Insert at the end. A folded Prompt is unfolded first (FOCUS.md: an explicit jump restores), even when that moves Review to a narrower width tier; Review keeps its cursor and file, and `␣W]` folds the Prompt again after sending. Nothing is sent to the agent until `Mod+Enter`. `␣VS` never unfolds |
| `Esc` | Leave Visual / cancel a pending key / close the hover; otherwise nothing |

In a diff, a location names file lines, new side first, not diff rows
(13-code-review). `*`, `#` and `/` search the buffer; worktree-wide text
search is `␣VT`.

Collision policy additions (both audits, `chromeShortcutPolicy.ts` and
`macShortcutPolicy.ts`): `Ctrl-O` overrides Chrome's Open File only inside the
focused Document, as a standard Vim motion in an exclusively owned context —
the same clause that admits Conversation's `Ctrl+D/U/F/B`. `Ctrl-I` is `Tab`,
which a read-only buffer does not use. `K`, `gd`, `gt`, `]c` are bare letters
inside an owned Vim context and need no entry.

### Navigator: Cowboy list grammar

The Navigator is one mounted owner with four MUI `Tabs` — Changes, Files,
History, Worktrees — and keeps the Mobile semantics for each (comparison,
unread filter, fetch, reviewed marks, commit drill-in, worktree ownership,
pull requests). Keys are the shared list contract; the key strip at its foot
names them while it owns focus.

| Key | Action |
| --- | --- |
| `J/K` `gg/G` `Ctrl-D/U/F/B` `zz zt zb` | Motions |
| `'` | Letter labels on the visible rows (`DesktopHintLayer`) |
| `[` / `]` | Previous / next Navigator tab (the modal tab grammar) |
| `/` | Filter (Insert; `Esc` returns to the list). In Files the row holds `Files | Text` as an MUI `ToggleButtonGroup`; `H/L` switch it while the field is in Normal |
| `L` / `Enter` | Open in the preview file tab (Changes: the diff; Files: the source; History: the commit's file list; Worktrees: switch to the owning Session, or Review the worktree) |
| `H` | Files tree: collapse, or move to the parent; History: back from a commit |
| `za zM zR` | Files tree folds, as in the Sessions tree |
| `␣VM` / `␣VS` | Mark the focused row's file reviewed; send its path to the Prompt (leader actions targeting the focused row) |

Changes keeps the complete queue for `]f`/`[f` and mounts rows in windows, as
Mobile does. The Changes header shows the comparison button (`␣VA`), unread
toggle (`␣VU`), fetch time and counts; a file row shows status, path shortened
from the left, `+n −m` and its reviewed mark.

A file row also carries a **problem badge** in its trailing slot: the count of
that file's mapped problems (AI findings at `exact`/`heuristic`, check
findings on verifiable lines, language errors), coloured by the highest
severity (error/critical-high in `error`, medium in `warning`, low in `info`
theme tokens), with an accessible label such as "2 findings, highest high".
`unmapped` and stale findings are not counted, so the badge never points at a
line the Document cannot show; they stay in the Inspector's Problems with
their state. Rows without problems reserve no space. The badge is a read-out,
not a control, so it has no slot; `]d` inside the file and the Problems list
are the actions. This lets a reviewer choose the next file by risk without
opening each one.

### Inspector: what is beside the code

| Section | Content | Keys |
| --- | --- | --- |
| Outline | Document symbols (`outline@1`, exact or heuristic, labelled) | `Enter` moves the cursor; `gO` arrives here |
| Hunks | Hunk list with `@@` headers and `+n −m` | `Enter` jumps; `]c` keeps the row in step |
| Problems | Language diagnostics, check findings (dated per run) and AI findings (severity, title, call, anchor state) for the open file | `Enter` jumps; `␣VS` sends the focused problem with its location to the Prompt |
| Details | Path, status, comparison, file facts, worktree revision, the latest review call's verdict and completeness | read-out |

`J/K` move rows, `H/L` fold a section, `'` labels rows. Sections are MUI
`List`s with `ListSubheader`s, not Accordions.

### File tabs

File tabs reuse the Desktop tab-strip primitive (recessed rail, italic preview
tab, pinned tabs first) but not its Session commands. The strip is not a focus
region and its tabs are `tabIndex=-1`, so the file-scoped commands are the
`␣VB` group above plus `gt`/`gT` in the Document, each drawn as a slot on the
strip's trailing control and listed in the tab's context menu. A file tab is
kept (made permanent) by `␣VBK`, pinning, moving or a double click; typing
cannot keep a read-only tab, so the keep command is not optional here.

### Status line

While Review is shown the status line adds a `REVIEW` segment beside the mode
(the Document's Vim mode while it owns focus: `NORMAL`, `VISUAL`), names the
focused region (`Review · Document`), and shows the comparison (`main ↔ Ready
to merge`), reviewed count (`3/51`), the open file's `+n −m` and freshness
("worktree changed 4 s ago" while a refresh is pending). Its legend lists the
Document keys (`]c Hunk`, `]f File`, `⏎ Send`, `K Hover`, `gd Def`, `gO
Outline`) or the list keys, and `␣V More`.

## Editor engine: CodeMirror 6 stays

The question was whether the Document should move to VS Code's components —
in practice the Monaco editor and its diff editor — or stay on CodeMirror 6.

| Criterion | Monaco (VS Code editor) | CodeMirror 6 + `@codemirror/merge` |
| --- | --- | --- |
| Already in Cowboy | No. New engine beside the Composer's CM6 | Yes: Composer, Drafts, Mobile Review, `cmTheme`, language loading, `codeSyntaxTheme` |
| Vim | `monaco-vim`: a second Vim implementation and config path | `@replit/codemirror-vim`, already configured by the user's vimrc subset |
| MUI | Own theme engine and widgets (find, hover, menus) outside MUI tokens | Theme from MUI tokens through `cmTheme`; widgets are MUI components |
| Bundle | ≈ 3–4 MB plus workers in a cache-first PWA whose boot asset list is precached | `@codemirror/merge` ≈ 30 KB on top of what Mobile already loads |
| Side-by-side | Built in, needs two complete models | `MergeView` (aligned chunks, collapse unchanged, `goToNextChunk`) and `unifiedMergeView` |
| Data model | Full documents; conflicts with 256 KiB diff pages, windowed files and revision fences | Paged document with decorations today; merge views accept any two documents |
| Intelligence | Browser-side workers; Cowboy's comes from the Code Review Engine | Server-provided hover/definition/diagnostics, rendered as decorations |
| Mobile | Already rejected (13-code-review); Desktop shares the chunk graph | Shared grammar, hooks and extension builders |
| What it would add | Minimap, sticky scroll, inline move detection, accept-change widgets | Minimap and sticky scroll are not needed by a Vim user with `]c` and the Hunks list; accept/reject is editing, out of scope |

**Decision: CodeMirror 6, adding `@codemirror/merge` for the split layout.**
One editor engine, one Vim, one theme, no new worker, and the bounded reading
model survives. Monaco would buy a polished split view at the price of a second
engine that does not speak Vim, MUI or Cowboy's paging.

### Split view: patch first, sides only when bound

A diff snapshot is paged at about 256 KiB, cut at a newline where one is near
(`src/diff_snapshot.rs`); a page therefore does not end on a hunk boundary,
and a large replacement can have its removals on one page and its additions on
the next. Mobile appends pages of one revision into one text document and
decorates it, which tolerates that. The split layout needs hunk structure, so
it uses a **streaming patch decoder**: parser state carried across pages of the
same revision, incomplete lines and hunks buffered until the next page or the
terminal truncation marker, continuation distinguished from truncation, file
headers, `rename from/to` and `Binary files … differ` kept as file metadata
outside the hunk documents, and `\ No newline at end of file` and CRLF carried
per line. Fixtures cover a hunk and a single line crossing a page boundary, a
rename-only file, a binary row, a truncated diff and an empty hunk before D2
acceptance.

From decoded hunks the split layout builds two aligned documents per hunk
(`-` lines left, `+` lines right, context on both) for `MergeView`, with the
unchanged regions collapsed — the way GitHub renders a patch. Word-level
highlights (`diffWordModel`), context folds, check annotations and the line
gutter are shared extension builders used by both layouts.

**Expanding context is not reconstructed from the worktree.** `GET /file`
reads today's file, whose metadata revision is paging evidence and not bound
to the diff snapshot ([plugin-review-owned-diff](plugin-review-owned-diff.md)
disclaims old-side identity); a historical, staged or `committed:` comparison
has no worktree side at all, and an ignore-whitespace patch cannot be reversed
exactly. Reverse-applying hunks over a later worktree read would fabricate
old-side text. Expanded context therefore requires **comparison-bound side
windows** from the server (see "Server and API"): the old side from the exact
base tree object of the snapshot's comparison and the new side from the exact
object or identity the snapshot diffed, each windowed like `/file` and refused
once the snapshot is gone. Until a Machine serves them, the split layout stays
hunk-aligned and its status row says "Context limited to the patch";
ignore-whitespace diffs never offer expanded context.

Source windows, large files, images, Markdown preview and Mermaid keep the
Mobile contracts (first 256 KiB window, continuation by revision, 32 MiB
preview cap, plain text first while the grammar loads).

## MUI composition

| Surface | MUI | Notes |
| --- | --- | --- |
| Work-pane header | `Tabs` (`Conversation`, `Review`) with leader slots; action rail of `IconButton`s | The transcript stays mounted behind |
| Approval dock | unchanged `DesktopApprovalDock`, hosted by the work pane | below either view |
| Navigator tabs | `Tabs` (dense, `desktopSize` heights) | `[`/`]`, `␣VC/F/H/W` |
| Change rows, tree rows, Inspector rows | `List`, `ListItemButton`, `ListSubheader`; `data-desktop-item` | Selected and cursor states per the Sessions rule: tinted fill for the open file, crisp outline for the cursor |
| Comparison, context, checks | `Button` + `Menu` with the Vim menu grammar (`menuNavigation.ts`) | `␣VA`, `␣VX`, `␣VK` |
| Unread, wrap, whitespace, layout | `ToggleButton` / `ToggleButtonGroup` | `␣VU`, `␣VD` |
| Status rows over a document | one-line `Alert` (dense) with a Details disclosure | 13-code-review's one-line rule |
| Hover | `Popover` with Markdown, anchored at the cursor | `K`, dwell when enabled |
| Selection footer | `Toolbar` of `Button`s: Send `⏎`, Explain `␣VE`, Review `␣VR`, Mark `␣VM` | always mounted; disabled explanations |
| File tabs | the Desktop tab-strip primitive with the `␣VB` group and `gt`/`gT` | context menu lists the same commands |
| Splitters, keycaps, status line, editor | Cowboy primitives | the documented exceptions |

Colors, borders and state layers come from theme tokens; both themes are
verified. Geometry follows `desktopSize`/`glyphSize`, first-level rows use
`DESKTOP_SURFACE_RADIUS`, nested chips `DESKTOP_INSET_RADIUS`, and the font
scale audit covers every new control at 100 % and 150 %. Code typography stays
Review's own 12/14/16/18 px setting. Network actions (fetch, run check, mark
reviewed, AI review) use `NetworkActionFeedback`.

## State ownership

The synced `mobile-review:<sid>` state is split by meaning, not by product.
Desktop reads the review's identity and progress and writes only those; it
never writes `tabs`, `active` or `mode`.

| Field | Owner | Why |
| --- | --- | --- |
| `comparison`, `bindings`, `remote_review`, `remote_selected`, `directory` | synced (Session) | What is being reviewed; agents set it with `setComparison`, `bind`, `unbind` through the sync operation |
| `progress` (reviewed marks, keyed by change fingerprint) | synced | A review's progress is the same on every device |
| `positions` (last line per path) | synced, last writer wins | Continue on Desktop where the phone stopped |
| `open {path, reveal}` | synced request, consumed by the reveal adapter | Agents and links ask the current device to show a file |
| `tabs`, `active`, `mode`, folds, split/unified, pane widths, work view | Desktop device-local (`desktopReviewState.ts`, `localStorage`) | Like Desktop tabs and pane folds; the phone's tabs are its own |

### Reveal

`open` is Mobile's navigation: on the server (`src/core/hub_sync.rs`) it
pushes a tab, sets `active` and `mode`, bumps `open_revision` and, with
`reveal`, bumps `reveal_revision`; `useReviewOpenRequest` observes only the
counter. Desktop does not reuse `open` for its own navigation (that would move
the phone's tabs) and does not follow `active` in general. Its **reveal
adapter** reads, at the moment `reveal_revision` increases, the `active` path
and the state's context (`directory`, bindings) from the same patch, and opens
that path in Desktop's local preview tab; a second reveal before the first is
painted replaces it. `open` with `reveal` now also records `reveal {path, line?, revision}`
(`src/core/mobile_review.rs`), bound to the `reveal_revision` it set, and
`sessions.review_open` takes an optional 1-based `line`. The adapter
prefers that payload (`revealedFile`) and reads `active` only when the state
comes from a writer that predates it; a line moves Desktop's cursor without
taking focus. The context stays the Session's own worktree, so the payload
names no context.

Device-local layout state is UI preference and is not exposed through Cowboy
MCP, the same documented exception as `cowboy:desktop-collapsed-panes`. No new
Service or Device setting is added.

## The agent loop

Review can show code the active Agent is not working in: a linked session, a
project checkout, an unowned worktree, a Remote PR (13-code-review "Change
target and worktree ownership"). Every handoff therefore carries the **browsed
context**: context id (`session::`, `workspace::`, `worktree::`, remote PR),
Machine, root, comparison and the diff or file revision. A remark names the
root when it is not the Session's own worktree, as `reviewExplainPrompt`'s
`viewedRoot` does today; an identical relative path in two roots never
collapses into one.

1. **Remark.** `Enter` or `␣VS` puts `path:line-range` (with the root when it
   differs) and the lines into the Composer; `Mod+Enter` sends. Several `␣VS`
   collect into one message.
2. **Ask.** `␣VE` forks the agent (`latest_completed`, one live Explain per
   session with `reuse`, 13-code-review "Explain"). Before the fork opens as a
   preview tab, the source Session's tab is kept: a Session opened from the
   sidebar and only reviewed is usually still the single preview tab, which
   the next preview would replace (FOCUS.md "Tabs", `workspaceTabs.ts`). An
   existing live fork's tab is reused. The fork's Conversation is shown; `␣⇥`
   returns to the review.
3. **Delegate.** `␣VR` asks that fork to start a managed review; a reader has
   no call authority of its own. The fork's first message is a fixed
   **delegation contract**, not prose: `review.start` with `task` (a stable id
   derived from the source Session, context and scope), `base` (the browsed
   root's full `HEAD` SHA from `GET /repository`), `root` (the browsed root),
   `files` (the scope, at most 32), `focus` (the selected lines and the
   question), `notify: true`. `review.start` selects the structured findings
   schema only when `task` and `base` are both given, and a base-bound review
   needs `root` (`agent_api/actions.rs`); without them the result is prose and
   Review shows it as text, never as findings.
4. **Find the call.** Calls are listed and read through their actual parent,
   the fork, and the source Session's call list does not include descendants
   (`managed_calls.rs`). The Controller therefore provides a parent-scoped
   **findings projection** (see "Server and API") that resolves the review
   calls of a Session and of its live Explain forks (Trash included) by the
   fork record and the `review_task` label. A deleted fork removes its calls;
   Problems says "review removed with its session". Repeated questions are
   told apart by task id, never by "the latest call".
5. **Read findings where they apply.** A finding is drawn on a line only when
   the projection vouches for it. `input_revision` is an opaque digest of
   HEAD, index, worktree and requested files (`managed_calls/snapshot.rs`),
   not a file identity, so the projection maps each finding from the call's
   snapshot to the displayed file: `exact` when the snapshot's blob for the
   path equals the current side's content identity, `heuristic` when lines
   can be carried through the snapshot→current diff (labelled, per 19's
   precision vocabulary), `unmapped` otherwise (listed, not drawn). Findings
   are consumed only from a result with `review_complete: true`, a verdict and
   no truncation; anything else is shown as an incomplete review with its
   text. `]d` steps mapped findings, `␣VS` sends one back as a remark.
6. **Check.** `␣VK` runs a declared project check through the session's own
   execution keeper; it is enabled only while the browsed context is the
   active Session's own worktree and `GET /checks` reports the check runnable,
   keeps the epoch and operation id across an uncertain reply, and its
   anchored diagnostics join Problems, dated per run.
7. **See the change land.** The worktree revision poll refreshes Changes and
   marks the open diff "no longer matches" in a status row; `]f` continues.

## Server and API

The first two phases use `changes`, `diff`, `file`, `tree`, `search`,
`content-search`, `checks`, `repository`, the language routes and the manifest
poll as they are. Three additions are dependencies of later phases, each
staged reader-before-writer with optional fields and explicit refusal from a
Machine that predates it:

| Addition | Phase | Contract |
| --- | --- | --- |
| Comparison-bound side windows: `GET /file?path=&side=base\|new&diff=<snapshot revision>&cursor=` (implemented as `GET /blob?oid=`, see D2d below) | D2b | Windowed like `/file` from the exact objects the snapshot diffed; `410` once the snapshot is gone; never for ignore-whitespace snapshots. Required for expanded context in the split layout |
| Findings projection: `GET /api/code/sessions/{id}/findings` and `?call=&path=&revision=` | D3 | Lists completed review calls of the Session and its live forks (call, task, base, root, files, verdict, completeness); per path returns anchors with `state: exact\|heuristic\|unmapped` computed from the snapshot blob and the current content identity. Authority: the reader must already see both sessions; reuses the call detail checks |
| `open` reveal payload: optional `reveal {path, context, revision}` on `mobile-review:<sid>` | D4 | Written by new `open` writers; Desktop's adapter prefers it over the narrow `active` read |

## Code ownership

```text
web/src/review/                 shared domain, no UI (moved from mobile/review)
  codeApi.ts  diffCache.ts  diffNavigationModel.ts  diffWordModel.ts
  diffContextModel.ts  directoryTreeModel.ts  outlineModel.ts
  reviewCompareModel.ts  repositoryTargetModel.ts  reviewContextModel.ts
  checkAnnotations.ts  diffCheckAnnotations.ts  reviewChecksStore.ts
  useOwnedReviewBuffer.ts  useOwnedReviewDiff.ts  useReviewRecovery.ts
  codeLanguage.ts  codeSyntaxTheme.ts  reviewLineSelection.ts
  reviewReadTelemetry.ts  extensions/ (gutter, word diff, folds, annotations)
web/src/desktop/review/         Desktop UI
  DesktopReviewView.tsx          work-pane view, header tabs, width tiers
  DesktopReviewNavigator.tsx     Changes / Files / History / Worktrees
  DesktopReviewDocument.tsx      CM6 host: source, unified, split; file tabs
  DesktopReviewInspector.tsx     Outline / Hunks / Problems / Details
  desktopReviewState.ts          device-local view state, reveal adapter
  reviewCommands.ts              the ␣V group and region commands
  reviewVim.ts                   editor-scoped read-only Vim additions
  reviewPatchDecoder.ts          streaming decoder over revision-matched pages
  reviewMerge.ts                 hunk-aligned split documents
  reviewFindings.ts              findings projection client, anchor states
  reviewDelegation.ts            the fork's review.start contract
web/src/mobile/review/          Mobile UI, unchanged behaviour
```

Rules: the shared move is a mechanical slice recorded with
`bun tools/check-structure.ts --update`; no offender grows; Desktop never
imports `ReviewApp`, `DocumentView` or `CodeViewer`; Mobile never imports
`desktop/review`; the Desktop pane model gains `review` as a work pane
(`DesktopPane`), and the invariant "Prompt and Conversation never collapse
together" reads "Prompt and the work pane".

## Phasing

| Phase | Scope | Acceptance |
| --- | --- | --- |
| D0 seam (implemented) | Work-pane view tabs; `review.open`/`review.focus` and the gated `␣V` group; `REVIEW` status segment; `Esc` policy; `DesktopPane` `review`; the approval dock hosted by the work pane; FOCUS.md updated; the current `ReviewApp` still renders inside | `just desktop-browser-gate`; shortcut audit lists no uncovered control; an approval arriving while Review is open is answerable at the pane's full width; Mobile screenshots and chunks unchanged |
| D1 workbench (core implemented) | Width tiers, Navigator with list grammar and tabs, Document as read-only Vim buffer (unified diff, source, file tabs, `]c ]f ]u gt`, `Enter`/`␣VS`), editor-scoped Vim additions, device-local state and reveal adapter, comparison/unread/mark | Keyboard-only scripted review of a 50-file change at 1100, 1440 and 1728 px; no synced tab writes from Desktop; Composer and Review mounted together with a vimrc mapping of `]c` still owned by the user; `H` across width tiers without remount |
| D1b (implemented) | History (commit drill-in, commit tabs) and Worktrees tabs, the context control `␣VX` (this Session, project checkout, linked, bound and same-project sessions; a per-device choice), Review settings `␣V,`, a resizable Navigator in Resize mode | `tools/cdp-review-acceptance.ts` (15 trusted-input steps, both themes) |
| D1c (implemented) | Remote PR in the workbench: the Pull request tab (`␣VG`) binds a PR through the synced `remote_review`, lists its files and opens them as in-memory `pr` tabs with viewed marks in the synced progress; Desktop ignores the phone's `remote_selected` | the acceptance's Pull request step against an opt-in GitHub fixture |
| D2a (implemented) | Inspector (Outline, Hunks, Problems, Details) beside or inset, `]d`/`[d`, `gO`, `␣VO`, `␣V]`, `␣VK`; Problems are check findings, placed only on source lines or through the verified diff projection; language diagnostics wait for D2c's buffer leases | acceptance Inspector steps (check, `]d`, `gO`, inset at narrow width) |
| D2b (implemented) | Split diff (`␣VD`, per device): an incremental decoder (`unifiedPatchDecoder.ts`) and an aligned two-sided layout (`splitDiffModel.ts`); the new side is the Vim buffer, the old side a passive mirror that follows its scroll; selections, `]c`, the Inspector and remarks map split rows to patch rows | decoder tests at every cut offset; acceptance split step |
| D2c (implemented) | Language intelligence on source files in the manifest's buffer mode: owned buffers through `useOwnedReviewBuffer` (every answer checked against the shown text), legacy through a buffer lease and the language routes; `K` hover, `gd` `gD` `gy` `gI` `gr` (legacy only, as on the phone; owned reports navigation unavailable), diagnostics underlined and merged into Problems; a jump keeps its source tab. Diffs point to the file | acceptance language step (diagnostic, `K`, `gd`, `gT`) |
| D2 tools | Streaming patch decoder and `@codemirror/merge` split view (patch-only), Inspector (Outline, Hunks, Problems, Details), `K` `gd` `gr` `gO` `]d`, checks `␣VK` with context gating, Review settings | Decoder fixtures pass; split and unified agree on every hunk of the fixture; hover/definition round trips under the measured `review_read_completed` budget |
| D2d sides (implemented) | Comparison-bound side windows; expanded context in the split layout | Expanded old-side text equals `git show <base>:<path>` for the fixture; an edited worktree never changes it |
| D3 (implemented, Web projection) | Delegation contract, Explain/Review forks as kept-source preview tabs, findings projection with anchor states, `␣VS` on a problem | A managed review's findings land on their lines at `exact`, move to `heuristic`/`unmapped` after an edit, and disappear with a deleted fork; a prose result is shown as text |
| D4 isolation (implemented, see notes) | Desktop drops `mobile/review` UI imports; shared domain under `web/src/review/`; `open` reveal payload; 13 updated | Structure ratchet totals not above baseline; Mobile unchanged |

D1 as implemented (`web/src/desktop/review/`): the Navigator has Changes and
Files; the Document is the read-only Vim buffer with unified diffs, source
files, file tabs and the footer (path, `+n −m`, hunk position, Mark reviewed,
Send to Prompt). Two deviations from the table above: the leader has no
nested groups, so file-tab commands are `␣VQ` close and `␣VP` keep instead of
a `␣VB` sub-group, and Review settings stay in the classic layout. The
Desktop code font is at least 12 px, so the phone's smaller sizes in the
shared Review settings do not shrink it. A context chosen with `␣VX` is
stored per device for the Session (`cowboy.desktop.reviewContext.v1`) and
resolved like the folder's review directory; it never writes the synced
`directory` default. After a switch, focus moves to the new context's
Changes, because the menu's own focus restoration would land on a row the
switch unmounts. The context is the Session's worktree
or its folder's review directory; an agent's reveal opens a file only in the
Session's own worktree. `tools/cdp-review-acceptance.ts` drives the
workbench with trusted keys in both themes and is part of
`just desktop-keyboard-acceptance`.

D2d as implemented reads the old side by the blob id the diff's own
`index` line names, `GET /api/code/sessions/{id}/blob?oid=`, instead of a
snapshot-bound `/file?side=&diff=`. A blob id is content-addressed, so the
answer is the exact text the patch was computed from without a snapshot
fence or a TTL. The prefix is resolved among objects only (`rev-parse
--disambiguate`, never a ref of the same name), must match exactly one blob,
and replacement refs are ignored. A Machine's refusal keeps its status
(404/413/415); only a missing operation or route is 502.
Outside the hunks both sides are equal, so the old side alone fills every
gap: `␣VZ` (a per-device choice, `cowboy.desktop.reviewDiffContext.v1`)
shows working-tree and commit diffs with the whole file, each gap joined to
the hunk before it so `]c` lands where it did. The Web checks every context
and removed line against the blob and keeps the patch on any mismatch, as it
does for an ignore-whitespace or cut-short diff, a new or binary file, a PR
(its blobs live on the host), a side over 4 MiB and a Machine without the
route; the footer then says "Patch context only". Remote Machines answer
only after a Machine release carries the operation.

D3 as implemented deviates from "Find the call" and "Read findings where
they apply": there is no Controller findings projection yet, because a call
records only `input_revision`, not the per-file blob identity `exact` needs.
Desktop keeps the projection in the Web instead. `␣VE` and `␣VR` (and the
footer's Explain and Review) fork the Agent as the phone's Explain does;
`␣VR` sends the delegation contract with a task id
(`desktop-review-<time>-<random>`) and records the delegation on this device
(`cowboy.desktop.reviewDelegations.v1`: source, fork, task, files with their
change fingerprints). The last five are read through their forks
(`/api/sessions/{fork}/calls`, matched by the `review_task` label), polled
every 15 s while one runs and only while Review is shown; showing Review
again re-checks settled ones, so a deleted fork's findings go with it. A
fork with no matching call after 10 minutes counts as never started. Only a
`completed` call whose result is complete, untruncated and `approve` or
`needs-attention` yields findings. The delegation always names the browsed
worktree as `root`, because a reused fork keeps the worktree it was forked
with. A finding is `current` while its file's change
fingerprint equals the delegated one, `stale` after the file changed and
`unmapped` outside the delegated files; only `current` findings are placed,
and only where Problems already places findings (a source file, or a diff
through the verified projection). Each is listed with its reason otherwise.
Calls now record their snapshot blobs: the Machine's snapshot names each
requested file's blob in the captured tree (`ChildPrepared.files`), the
Controller keeps it on the call record (`input_files`, at most 32), and
`GET /api/code/sessions/{id}/file-blob?path=` returns the worktree file's
blob as `git add` would store it. When both are known a finding is `current`
exactly while the two blobs are equal (a file edited back to the reviewed
content is current again) and `stale` otherwise; the blob is read again when
the file's change fingerprint moves. Calls prepared by a Machine that
predates `files`, and contexts whose Machine lacks `file-blob`, keep the
fingerprint rule above, which is metadata and weaker. The recording takes
effect with the next Machine release; until then every call uses the
fingerprint. A Controller older than this change cannot decode a call record
that carries `input_files`, so a Controller rollback past it must also roll
back the Machine or accept those calls as unreadable. `heuristic` carrying
through an edit is still not done. The Navigator badges a
file with the count of its current findings, coloured by the highest
severity. Delegations are device-local, so another device does not see them.
`␣VS` has no problem-row form: `Enter` on a problem moves the Document's
cursor to it, and `Enter` there sends the line.

With D1c the workbench covers everything the classic layout offered on
Desktop, and D4 retired that layout with `␣VL`: Desktop renders only the
workbench and never loads `ReviewApp`.

D4 as implemented moves the shared review domain to `web/src/review/` (62
files with their tests, a mechanical move recorded with
`check-structure --update`): the Code API client, diff, owned-buffer and
owned-diff projections, review context, comparison, checks, settings and
progress stores, remote PR sources and models, and the language and syntax
helpers. `readRemoteReview` moved from `RemoteReviewApp.tsx` into
`remoteReviewSources.ts`. The shared domain imports nothing from
`mobile/` and Mobile never imports `desktop/review`. Three parts of the
plan remain open: Desktop still renders `ReviewSettingsContent` from
`mobile/review` inside its settings modal (a settings form, not a review
surface; it moves when a second Desktop settings form needs it), and the
pane model keeps D0's three panes, with Review as the work pane's view
rather than a `DesktopPane`. The `open` reveal payload landed after D4 (see
"Reveal").

D2b deviates from "Editor engine": it does not add `@codemirror/merge`. A
patch-only split needs no diff algorithm, because the alignment comes from
the patch (deletions face the additions that follow them, the shorter side
padded), and the dependency would change the locked web inputs. Two
CodeMirror views carry the sides; both get the file's grammar, since the
split rows hold plain code. The loader joins only revision-matched pages
before the decoder runs, and the decoder itself is incremental and tested at
every cut offset, so a page boundary inside a hunk or a line cannot change
the result. Expanded context remains D2's later step (comparison-bound side
windows); until then the split shows the patch's hunks only.

D0 keeps the Desktop pane model at three panes: Review is the region
`conversation.review` of the Conversation pane rather than a fourth
`DesktopPane`, which leaves pane collapse, Resize mode and region motion
unchanged. The pane is renamed only if D1's workbench needs a distinct pane
scope.

## Verification gates

- `bun run typecheck`, `bun run lint` (structure ratchet), `bun run test`;
  `just desktop-browser-gate` for every key, label, modal or status change
  (exit 3 is "not run", never "passed"); `tools/cdp-shortcut-audit.ts`;
  `tools/cdp-icon-scale-audit.ts` and `fontScaleAudit` at 100 %/150 %.
- Both collision audits with the `Ctrl-O` Document entry and tests.
- Both themes, Desktop widths 1100/1440/1728/ultrawide, Review widths across
  the three tiers, long paths, empty/loading/error states, IME composition
  never reaching a Review key.
- Decoder fixtures (page-split hunk and line, rename-only, binary, truncated,
  empty hunk, missing final newline, CRLF); findings anchor states; the
  Explain preview opened from an initially unkept Session tab.
- `just desktop-perf-profile` scenarios: open Review, `]c` ×20, `]f` ×20,
  split toggle; no long task regression against the baseline.
- Mobile 390 px regression: no Desktop review chunk loaded.

## Decisions (r2, with r3 owner answers)

1. Review is a persistent view of the work pane, selected by `␣V`/`␣C` and
   header tabs; `Esc` never leaves it.
2. `␣V` is a group whose commands are gated on Review being shown;
   `review.open` is the root fallback, `␣VV` focuses the Document, `␣C`
   returns.
3. The Document is a real read-only Vim buffer (codemirror-vim) with Review's
   additions (`]c ]f ]u ]d gt K gd gr gO`) dispatched per editor, user
   mappings first.
4. `Enter` sends and focuses the Prompt; `␣VS` sends and stays; both carry the
   browsed context.
5. CodeMirror 6 stays; `@codemirror/merge` is added; Monaco is not.
6. The split view is built from a streaming patch decoder and shows the patch
   only until comparison-bound side windows exist; nothing is reconstructed
   from the worktree.
7. Synced review state is split by meaning; Desktop never writes `tabs`,
   `active` or `mode`; reveal is consumed through an adapter.
8. No bare product letters on rows: Mark and Send are `␣VM`/`␣VS` with the
   focused item as target; `␣VA` is the comparison, `␣VB` the file tabs.
9. `]f`/`[f` walk the complete queue; `]u`/`[u` walk unreviewed files.
10. Managed review from Review uses a fixed `review.start` contract (task,
    base, root, files, focus); findings are drawn only through the
    provenance projection and only from complete results.
11. The approval dock belongs to the work pane, visible under either view.
12. Explain opens the fork beside a kept source tab and reuses a live fork's
    tab.
13. `Enter` unfolds a folded Prompt; Navigator file rows carry a problem
    badge of mapped problems only (r3).

## Owner decisions on the r2 open questions

1. `Esc` in the Document never leaves Review: it is too frequent while
   reading code to double as a product-mode exit. Leaving is `␣C`.
2. `Enter` unfolds a folded Prompt before appending, accepting the narrower
   width tier; `␣VS` stays in place and never unfolds.
3. AI findings (and other mapped problems) show as a severity-coloured count
   badge on Navigator file rows; unmapped and stale findings are not counted.

## Review record

Codex Astra Max design review of r1, 2026-11-08 (managed call
`call-53884a467577cfb02c34574f`, preset `astra-max`, input revision
`sha256:4e9be292…`): verdict *needs-attention*, five high and ten medium
findings. Static review only; no build, browser or physical IME check was run.
Each finding and its disposition in r2:

| # | Severity | Finding | Disposition |
|---|---|---|---|
| 1 | high | Reverse-applying hunks over `/file` fabricates old-side context: the file's metadata revision is not bound to the diff snapshot, ignore-whitespace patches are lossy, and `plugin-review-owned-diff` disclaims old-side identity | Accepted. Expanded context requires comparison-bound side windows (D2b); the split layout is patch-only until then; ignore-whitespace never expands |
| 2 | high | `input_revision` is an opaque snapshot digest, not a per-file identity; path+line matching can annotate unrelated text; 19 requires anchoring to file identity and side | Accepted. The findings projection is a D3 dependency with `exact`/`heuristic`/`unmapped` anchor states; nothing is drawn unmapped |
| 3 | high | The approval dock sits inside the hidden transcript and claims the Session, so a permission during Review has no visible surface | Accepted. The dock is hosted by the work pane under either view (D0) |
| 4 | high | Diff pages cut at ~256 KiB without hunk boundaries; rendering pages as hunks corrupts alignment; rename/binary, EOF newline and CRLF need explicit handling | Accepted. Streaming patch decoder with fixtures before D2 acceptance |
| 5 | high | `review.start` yields structured findings only with `task` + `base` (+ `root`); `reviewExplainPrompt` requests none, so the fork can return prose | Accepted. Fixed delegation contract; findings consumed only from complete, untruncated structured results |
| 6 | medium | `open` writes `tabs`/`active`/`mode`; the reveal hook sees only a counter; ignoring `active` loses the path, reusing `open` moves the phone's tabs | Accepted. Reveal adapter reads `active` narrowly at reveal time (documented exception); optional reveal payload in D4 |
| 7 | medium | Remarks, forks and checks must carry the browsed context, or a same-named path in another root and a check against the wrong Session follow | Accepted. Every handoff carries context id, Machine, root, comparison, revision; checks gated to the Session's own worktree with epoch/operation retained |
| 8 | medium | The fork's call is not listed under the source Session; "latest call" cannot associate repeated questions | Accepted. Controller findings projection resolves calls through the fork record and `review_task`; deleted forks remove their findings |
| 9 | medium | `defineAction`/`mapCommand` are global and would change the Composer and Draft editors | Accepted. Editor-scoped CM6 keymap with user-mapping precedence; simultaneous-editor tests |
| 10 | medium | A group opens whenever any command is runnable, so an enabled `␣VV` toggle defeats the root fallback; `␣VV` cannot both return and focus | Accepted. Grouped commands gated on `reviewShown`; `review.open` is the root fallback; `␣VV` focuses; `␣C` returns; FOCUS.md updated in D0 |
| 11 | medium | `M`/`S` are the shared Move/Settings item verbs | Accepted. Removed from rows; `␣VM`/`␣VS` target the focused item |
| 12 | medium | Compact `H` closed the inset while also meaning collapse/parent/back | Accepted. The list owns `H`; the inset closes only at the navigation root; `Esc` leaves the filter first |
| 13 | medium | The tab-strip primitive has no focus region and `␣B` is Session-only, so file tabs had no keyboard keep/pin/close/reopen | Accepted. `␣VB` group mirrors `␣B` for file tabs; comparison moved to `␣VA` |
| 14 | medium | The Explain preview would replace an unkept source Session tab | Accepted. The source tab is kept before the fork opens; a live fork's tab is reused |
| 15 | medium | `]f` depending on the Unread filter contradicts 13-code-review's complete-queue rule | Accepted. `]f`/`[f` over the complete queue; `]u`/`[u` for unreviewed |
