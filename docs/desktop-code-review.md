# Desktop Code Review

**Status: design proposal r1 (2026-11-08), not implemented.** Normative
contracts it builds on: [Desktop focus and keyboard](../web/src/desktop/FOCUS.md),
[Desktop efficiency](desktop-efficiency-redesign.md),
[core UX principles](ux-principles.md), the Code Review data plane in
[13-code-review](architecture/13-code-review.md), the capability engine in
[19-code-review-engine](architecture/19-code-review-engine.md) and agent code
intelligence in [20](architecture/20-agent-code-intelligence.md). Where this
proposal changes a sentence of FOCUS.md or 13, that document is updated in the
implementing change, not before.

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
├──────────┴───────────┴─────────────────────────────────────────────────────────┤
│ NORMAL  REVIEW·Document  main ↔ Ready to merge  3/51 reviewed  ]c Hunk  ␣V More │
└───────────────────────────────────────────────────────────────────────────────┘
```

The Sessions pane, the tab strip, the top bar, the Prompt and the status line
are unchanged. The work column (today: Conversation) gains a second **view**,
Review. Both views are mounted; switching never loses the transcript's reading
position or Review's cursor, folds and tabs.

### Inner width policy

Review measures its own container (`@container`), never the viewport:

| Review width | Layout |
| --- | --- |
| ≥ 1320 px | Navigator (persisted, default 280 px) · Document · Inspector (persisted, default 300 px) |
| 900–1319 px | Navigator · Document; the Inspector is folded. Restoring one side panel folds the other, so one side panel is open at a time |
| < 900 px | Compact: both side panels fold to 36 px rails; the Document has the width. `␣VC/F/H/W` open the Navigator as a temporary inset list (MUI `Drawer` anchored inside the pane, exclusive region, `Esc`/`H` or opening a file closes it) |

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
- **Return**: `␣C` (focus Conversation restores the Conversation view),
  `␣VV` inside Review, the header tab, or the palette.
- **`Esc` never leaves Review.** It unwinds Visual mode, a pending chord, the
  hover popover, the filter field or an inset list, and then does nothing, as
  in a Vim buffer. The earlier Desktop behaviour where Review's Agent action
  and Send to Agent return to the Conversation is dropped: the Prompt is
  already beside the code.
- **Agent-driven reveal** (`open {path, reveal}` on the synced state, used by
  agents and transcript file links) switches the view to Review and opens the
  file, but only as live navigation: hydration and session switches never
  steal focus (`useReviewOpenRequest.ts`, unchanged).
- The view is per Session and per device (`cowboy:desktop-work-view:<sid>`),
  so returning to a Session returns to where its review stopped.

### The `␣V` group

`␣V` becomes the Review group, following the group rule in FOCUS.md: a group
opens only when one of its commands can run; otherwise its key keeps the root
meaning. With Review closed only *open* can run, so `␣V` opens Review at once,
exactly as today. With Review open, `␣V` shows the group:

| Key | Meaning |
| --- | --- |
| `␣VV` | Review ⇄ Conversation; the doubled key also focuses the Document, as `␣SS`/`␣TT` focus their surfaces |
| `␣VC` `␣VF` `␣VH` `␣VW` | Navigator: Changes, Files, History, Worktrees — switch the list and focus it |
| `␣VB` | Comparison (base): Uncommitted, Ready to merge, Not pushed, This session, other branches, Fetch this branch |
| `␣VU` | Unread filter on the change list |
| `␣VD` | Diff layout: unified ⇄ split |
| `␣VM` | Mark the open file reviewed / unreviewed |
| `␣VS` | Send: append the selection (or cursor line) with its location to the Prompt and stay in Review (collect several, then write once) |
| `␣VE` | Explain the selection: a fork of the agent answers in a new tab |
| `␣VR` | Review the selection or open file with an AI reviewer (the fork starts a managed call) |
| `␣VK` | Checks: run a declared project check (`check@1`); a picker when several are declared |
| `␣VT` | Text search across the worktree (`search@1`): the Navigator filter in Text mode |
| `␣VO` | Outline: focus the Inspector's Outline section (`gO` from the Document) |
| `␣VX` | Context: project checkout, linked sessions, bind a worktree, Remote PR |
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

### Document: a read-only Vim buffer

The Document is CodeMirror 6 with `@replit/codemirror-vim`, the same engine,
vimrc subset (`vim/vimConfig.ts`) and `cmTheme` as the Composer, with
`EditorState.readOnly`. Every Vim motion, search, mark, count, register and
fold key therefore works unchanged, and the user's `map`/`noremap` lines apply.
Insert-mode entry (`i a o I A c s …`) is refused and the status line shows
Vim's `W10` wording, "Read-only". The buffer is one file: a unified or split
diff, or a source window.

Cowboy defines the following through codemirror-vim's `defineAction` /
`mapCommand` (so they are Vim mappings the user can remap), registered as
Desktop commands for discovery:

| Key | Action |
| --- | --- |
| `]c` / `[c` | Next / previous hunk (Vim diff mode) |
| `]f` / `[f` | Next / previous changed file in the Navigator's queue (next unread when the Unread filter is on); the ends stop and the status line says "Last change" |
| `]d` / `[d` | Next / previous problem in this file: language diagnostics, check findings and AI findings in one ordered set |
| `gt` / `gT` / `{n}gt` | Next / previous / nth file tab |
| `zo zc za zR zM` | Folds: collapsed unchanged regions in a diff, language folds in source |
| `K` | Hover (type, documentation) in an MUI `Popover` at the cursor; `K` or `Esc` closes, `j/k` scroll it |
| `gd` `gD` `gy` `gI` | Definition, declaration, type definition, implementation (LazyVim's LSP set): opens the target in the preview file tab and pushes the jump list |
| `gr` | References: the Inspector lists them, `Enter` jumps |
| `gO` | Outline in the Inspector (Vim's `gO`) |
| `Ctrl-O` / `Ctrl-I` | Jump list back / forward, across files |
| `V` `v` | Visual selection; a click on a line number selects the line and a Shift-click extends it, as on Mobile |
| `Enter` | Send the selection (or cursor line) to the Prompt: appends `path:lines` plus the text to the Composer and focuses it in Insert at the end. A folded Prompt is restored first (FOCUS.md: an explicit jump restores). Nothing is sent to the agent until `Mod+Enter` |
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
| `M` | Mark this file reviewed / unreviewed (same as `␣VM` on the open file) |
| `S` | Send this path to the Prompt (append, stay) |

Changes keeps the complete queue for `]f`/`[f` and mounts rows in windows, as
Mobile does. The Changes header shows the comparison button (`␣VB`), unread
toggle (`␣VU`), fetch time and counts; a file row shows status, path shortened
from the left, `+n −m` and its reviewed mark.

### Inspector: what is beside the code

| Section | Content | Keys |
| --- | --- | --- |
| Outline | Document symbols (`outline@1`, exact or heuristic, labelled) | `Enter` moves the cursor; `gO` arrives here |
| Hunks | Hunk list with `@@` headers and `+n −m` | `Enter` jumps; `]c` keeps the row in step |
| Problems | Language diagnostics, check findings (dated per run) and AI findings (severity, title, call) for the open file; stale ones labelled | `Enter` jumps, `S` sends the problem with its location to the Prompt |
| Details | Path, status, comparison, file facts, worktree revision, the latest review call's verdict | read-out |

`J/K` move rows, `H/L` fold a section, `'` labels rows. Sections are MUI
`List`s with `ListSubheader`s, not Accordions.

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

### Split view without a new endpoint

A unified diff page already holds both sides of every hunk. The split layout
builds two aligned documents per hunk window (`-` lines left, `+` lines right,
context on both) and feeds `MergeView` with the unchanged regions collapsed,
the way GitHub renders patches. "Expand context" reads the new side through
the existing windowed `GET /file` and derives the old side by applying the
hunks in reverse, which is exact while the diff is complete (not cut at the
16 MiB bound) and the file identity revision still matches; otherwise the
layout stays hunk-aligned and says so in its status row. A `GET /file?…&side=base`
window is an optimization for a later revision, not a dependency. Word-level
highlights (`diffWordModel`), context folds, check annotations and the line
gutter are shared extension builders used by both layouts.

Source windows, large files, images, Markdown preview and Mermaid keep the
Mobile contracts (first 256 KiB window, continuation by revision, 32 MiB
preview cap, plain text first while the grammar loads).

## MUI composition

| Surface | MUI | Notes |
| --- | --- | --- |
| Work-pane header | `Tabs` (`Conversation`, `Review`) with leader slots; action rail of `IconButton`s | The transcript stays mounted behind |
| Navigator tabs | `Tabs` (dense, `desktopSize` heights) | `[`/`]`, `␣VC/F/H/W` |
| Change rows, tree rows, Inspector rows | `List`, `ListItemButton`, `ListSubheader`; `data-desktop-item` | Selected and cursor states per the Sessions rule: tinted fill for the open file, crisp outline for the cursor |
| Comparison, context, checks | `Button` + `Menu` with the Vim menu grammar (`menuNavigation.ts`) | `␣VB`, `␣VX`, `␣VK` |
| Unread, wrap, whitespace, layout | `ToggleButton` / `ToggleButtonGroup` | `␣VU`, `␣VD` |
| Status rows over a document | one-line `Alert` (dense) with a Details disclosure | 13-code-review's one-line rule |
| Hover | `Popover` with Markdown, anchored at the cursor | `K`, dwell when enabled |
| Selection footer | `Toolbar` of `Button`s: Send `⏎`, Explain `␣VE`, Review `␣VR`, Mark `␣VM` | always mounted; disabled explanations |
| File tabs | the Desktop tab-strip primitive (recessed rail, italic preview, pin) | `gt`/`gT`; `␣B` acts on Session tabs only |
| Splitters, keycaps, status line, editor | Cowboy primitives | the documented exceptions |

Colors, borders and state layers come from theme tokens; both themes are
verified. Geometry follows `desktopSize`/`glyphSize`, first-level rows use
`DESKTOP_SURFACE_RADIUS`, nested chips `DESKTOP_INSET_RADIUS`, and the font
scale audit covers every new control at 100 % and 150 %. Code typography stays
Review's own 12/14/16/18 px setting. Network actions (fetch, run check, mark
reviewed, AI review) use `NetworkActionFeedback`.

## State ownership

The synced `mobile-review:<sid>` state is split by meaning, not by product.
Nothing changes on the wire; Desktop simply stops reading the device fields.

| Field | Owner | Why |
| --- | --- | --- |
| `comparison`, `bindings`, `remote_review`, `remote_selected`, `directory` | synced (Session) | What is being reviewed; agents set it with `setComparison`, `bind`, `unbind` through the sync operation |
| `progress` (reviewed marks, keyed by change fingerprint) | synced | A review's progress is the same on every device |
| `positions` (last line per path) | synced, last writer wins | Continue on Desktop where the phone stopped |
| `open {path, reveal}` revisions | synced request | Agents and links ask the current device to show a file |
| `tabs`, `active`, `mode`, folds, split/unified, pane widths, work view | Desktop device-local (`desktopReviewState.ts`, `localStorage`) | Like Desktop tabs and pane folds; the phone's tabs are its own |

Device-local layout state is UI preference and is not exposed through Cowboy
MCP, the same documented exception as `cowboy:desktop-collapsed-panes`. No new
Service or Device setting is added.

## The agent loop

1. **Remark.** `Enter` or `␣VS` puts `path:line-range` and the lines into the
   Composer; `Mod+Enter` sends. Several `␣VS` collect into one message.
2. **Ask.** `␣VE` forks the agent (`latest_completed`, one live Explain per
   session with `reuse`, 13-code-review "Explain"), opens the fork as a new
   preview tab beside the current Session tab and shows its Conversation; the
   fork has the context and its own worktree. `␣⇥` returns to the review.
3. **Delegate.** `␣VR` asks that fork to start a managed review of the
   selection or file (`review.start`); a reader has no call authority of its
   own. The call shows in Background tasks (`␣G`).
4. **Read findings where they apply.** When the call ends, its structured
   findings (`severity`, `title`, `body`, `file`, `line_start`, `line_end`)
   appear in the Inspector's Problems for the files they name and in the
   gutter, anchored to the call's `input_revision`. A finding whose file
   changed since is listed as stale and not drawn on lines. `]d` steps them,
   `S` sends one back to the Prompt as a remark.
5. **Check.** `␣VK` runs a declared project check through the session's own
   execution keeper; its anchored diagnostics join Problems, dated per run.
6. **See the change land.** The worktree revision poll refreshes Changes and
   marks the open diff "no longer matches" in a status row; `]f` continues.

Findings are read through the existing parent-scoped call detail endpoint;
Review adds no call route. Every read keeps the existing authority checks.

## Server and API

No Controller or Machine change is required for the first two phases.
`changes`, `diff`, `file`, `tree`, `search`, `content-search`, `checks`,
`repository`, the language routes and the manifest poll are used as they are.
Later optional additions, each a reader-first optional field or query:

- `GET /file?path=&cursor=&side=base`: a base-side window for a complete split
  view of large files (D2 optimization).
- A `findings@1` projection of a review call for a given file and revision,
  if client-side anchoring proves too costly (D3 optimization).

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
  desktopReviewState.ts          device-local view state
  reviewCommands.ts              the ␣V group and region commands
  reviewVim.ts                   read-only Vim buffer, ]c ]f ]d gt K gd gO
  reviewMerge.ts                 hunk-aligned split documents, reverse apply
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
| D0 seam | Work-pane view tabs; `␣V` group semantics; `REVIEW` status segment; `Esc` policy; `DesktopPane` `review`; the current `ReviewApp` still renders inside | `just desktop-browser-gate`; shortcut audit lists no uncovered control; Mobile screenshots and chunks unchanged |
| D1 workbench | Width tiers, Navigator with list grammar and tabs, Document as read-only Vim buffer (unified diff, source, file tabs, `]c ]f gt`, `Enter`/`␣VS`), device-local state split, comparison/unread/mark | Keyboard-only scripted review of a 50-file change at 1100, 1440 and 1728 px; no synced tab writes from Desktop |
| D2 tools | `@codemirror/merge` split view, expand context, Inspector (Outline, Hunks, Problems, Details), `K` `gd` `gr` `gO` `]d`, checks `␣VK`, Review settings | Hover/definition round trips under the measured `review_read_completed` budget; split and unified agree on every hunk of the fixture |
| D3 agent loop | Explain/Review forks as tabs, findings anchored from call results, `S` on a problem | A managed review's findings land on their lines at the call revision and go stale correctly after an edit |
| D4 isolation | Desktop drops `mobile/review` UI imports; shared domain under `web/src/review/`; FOCUS.md and 13 updated | Structure ratchet totals not above baseline; Mobile unchanged |

## Verification gates

- `bun run typecheck`, `bun run lint` (structure ratchet), `bun run test`;
  `just desktop-browser-gate` for every key, label, modal or status change
  (exit 3 is "not run", never "passed"); `tools/cdp-shortcut-audit.ts`;
  `tools/cdp-icon-scale-audit.ts` and `fontScaleAudit` at 100 %/150 %.
- Both collision audits with the `Ctrl-O` Document entry and tests.
- Both themes, Desktop widths 1100/1440/1728/ultrawide, Review widths across
  the three tiers, long paths, empty/loading/error states, IME composition
  never reaching a Review key.
- `just desktop-perf-profile` scenarios: open Review, `]c` ×20, `]f` ×20,
  split toggle; no long task regression against the baseline.
- Mobile 390 px regression: no Desktop review chunk loaded.

## Decisions taken in r1 (owner may revise)

1. Review is a persistent view of the work pane, selected by `␣V`/`␣C` and
   header tabs; `Esc` never leaves it.
2. `␣V` becomes a group; the root meaning (open) is preserved by the group
   rule, and return is `␣VV` or `␣C`.
3. The Document is a real read-only Vim buffer (codemirror-vim), with Cowboy's
   additions defined as Vim mappings: `]c ]f ]d gt K gd gr gO`.
4. `Enter` sends and focuses the Prompt; `␣VS` sends and stays.
5. CodeMirror 6 stays; `@codemirror/merge` is added; Monaco is not.
6. Split view is built client-side from unified pages; no new endpoint.
7. Synced review state is split by meaning: review identity and progress sync,
   device view state does not; the wire is unchanged.
8. Navigator rows use `M` for Mark reviewed and `S` for Send path, two
   region-scoped bare letters named in the key strip.
9. Explain forks open as preview tabs in the tab strip.
10. Side panels follow the one-open-panel rule below 1320 px and fold to rails
    below 900 px; Review never folds Sessions or Prompt.

## Open questions for the design review

- Should `Esc` in Document Normal mode return to the Conversation, as FOCUS.md's
  "then product mode" clause could be read, or stay inert as in Vim (r1)?
- Is `]f`/`[f` the right file motion, or should the queue use `]q`/`[q`?
- Should `Enter` restore a folded Prompt (FOCUS.md's jump rule) or only append
  and announce, given that restoring shrinks Review to Compact at 1440 px?
- Is the hunk-aligned split with reverse-applied context acceptable until a
  base-side window exists, including for renames and binary rows?
- Should AI findings also mark the Navigator rows (count badge) or stay in the
  Inspector and gutter?

## Review record

r1 awaits a Codex Astra Max design review; findings and dispositions are
recorded here in r2.
