# Desktop focus and keyboard contract

Desktop is a Vim-first productivity surface. Mobile does not load this focus
controller, command registry, or shortcut guide.

## Hierarchy

Keyboard focus has three levels:

1. pane: Sessions, Prompt, Conversation;
2. region: Top Bar, list/editor/transcript or Prompt's Plan, Queue, Drafts and Composer;
3. item: a session, queued prompt, draft, or transcript entry.

DOM integration uses `data-desktop-pane`, `data-desktop-region`, and
`data-desktop-item`. A region may mark its preferred real focus target with
`data-desktop-focus-default`. Mouse focus and keyboard navigation update the
same controller state.

The Top Bar is a workspace region, not a fourth pane. Its actions do not need
focus: `␣T` opens the Top bar group and the next key runs Run Configuration
(`R`), Usage (`U`), session verification (`A`), Reload (`L`), Compact (`C`)
or Clear (`X`) directly; each control shows its `␣TR`-style slot. Focusing
the bar (`␣TT`, or Ctrl-K from a pane) is mutually exclusive with Sessions and
Prompt: pane-header chrome clears, the bar itself takes the shared focus fill
plus a primary underline, and only the focused control receives the keyboard
cursor. Do not set `color: primary` on the Toolbar; `color="inherit"` children
would then all look selected. Ctrl-J returns to the pane that still owns
`focusedPane`.

## Core interaction laws

These rules are the canonical Desktop-mode primitive contract. New controls,
modals, product modes, and shortcut hints must reuse them rather than creating
component-local keyboard semantics.

### The primitive set

Every Desktop surface is built from the same six primitives. Learning them is
learning Cowboy; a new feature only picks which ones it uses.

1. **Modes.** A text field or Vim Insert owns its keys. `Esc`/`Ctrl-[` leaves
   it for Normal on the owning item or region; `i`/`Enter` goes back.
2. **Motions.** `J/K` vertical, `H/L` horizontal, `gg/G` ends,
   `Ctrl-D/U/F/B` pages, in every list, tree, reader, tablist and form.
3. **Item verbs.** On the focused item: `L`/`Enter` open, `I` edit or
   rename, `M` move, `S` settings, `O` order mode, `Shift-J/K` reorder.
4. **Leader.** `␣` then one key for everything else (below). Its slots are
   drawn on the controls they run.
5. **Labels.** When a choice is among many visible targets, the targets get
   letter labels for the next key instead of fixed numbers: `␣␣` opens a
   session, `'` (Vim's mark jump) moves the cursor to a row of the focused
   list, and the leader inside a modal labels every control of that modal.
   Labels are scoped to one list or one modal, appear only while armed and
   are painted by `DesktopHintLayer`; there is no page-wide hint layer.
6. **Direct chords.** Only established semantics: `Mod+Enter` send/confirm,
   `Mod+S` save, `Mod+.` stop, `Mod+Shift+P` palette, `Mod+/` help.

Components declare commands through `useDesktopCommand` (with a `sequence`
for leader keys) and draw slots with `LeaderKeycap`/`DesktopShortcut`; they
never read raw keys. `desktopKeyIntent` decides ownership first.

### Leader

`Space` (`␣`) arms the leader wherever Cowboy owns the key: Vim Normal,
lists, the reader, tablists and chrome. `Cmd+K` (macOS) / `Alt+K` arms the
same layer from Insert and native fields, where Space must type. Space never
arms inside text fields or native toggles, during IME composition (Space picks
a candidate), on auto-repeat, with a modifier (`Ctrl/Cmd+Space` switch input
sources, `Shift+Space` pages), inside an exclusive modal or menu, or in Resize
mode. A consumed Space also swallows its keyup, so a focused button is not
activated; `Enter` still activates buttons.

Once armed, like which-key it waits: `Esc`, a pointer press, window blur or
the next key ends it; `Backspace` climbs out of a sub-layer. After 180 ms the
which-key panel (`DesktopLeaderMenu`) appears bottom-right with every key that
runs in the current focus, the focused surface's own actions first under
"Here". At the same instant every on-screen leader slot lights
(`data-shortcut-state="active"`), so the panel is optional for a user who can
already see the key. Entries are clickable.

Groups follow LazyVim: a group key (`␣T`) opens a which-key layer whose
commands run from any focus, because the group is their scope; `Backspace`
returns to the root. A grouped slot is drawn as one keycap (`␣TR`) and lights
both at the root and inside its group. New families of actions become groups
rather than taking the last free root letters.

A leader slot is one keycap holding the glyph and the key: `␣N`, never
`⌘K → N`. At rest it is `available` while its scope owns focus and
`inactive` otherwise; armed, it is `active`. A leader key has one meaning;
scoped editors may each register it in disjoint regions (`␣/` in the Composer
and in a queued-message editor), and the focused one runs.

| Key | Meaning |
| --- | --- |
| `␣␣` | Switch session: rows get letter labels (home row first, flat displayed order); press one to open it and land in Prompt |
| ``␣` `` | Previous session |
| `␣N` | Create (Session / Draft / Folder) |
| `␣K` | Command Palette |
| `␣S` `␣P` `␣C` | Focus Sessions, Prompt, Conversation |
| `␣T` + `R/U/A/L/C/X/T` | Top bar group: run config, usage, verify, reload, compact, clear; `T` focuses the bar |
| `␣L` `␣Q` `␣D` | Focus Plan, Queue, Drafts |
| `␣W` + `W/R/[/]/\` | Window group: cycle regions, Resize mode, fold Sessions / Prompt / Conversation |
| `␣,` | Settings |
| `␣U` + `E` | Interface group: Source / live preview (Prompt) |
| `␣/` `␣F` `␣A` `␣Z` | Slash, reference file, attach, zoom (expand) the focused editor |
| `␣H` `␣J` | Schedule, run next (Composer) |
| `␣M` + `B/I/X/U/O/M` | Markup group in the focused editor: bold, italic, code, link, list, more formatting (`Mod+B`/`Mod+I` also format, as in Obsidian; off macOS only in Vim Insert or without Vim, so Ctrl-B/Ctrl-I keep their Vim meaning) |
| `␣D` + `R/V/H/E/W` | Draft group (an open Draft only): rename, copy to a Session, history, export, readable width; otherwise `␣D` focuses Drafts |

The root keeps what is pressed most (switching, creating, focusing, the
editor's insert actions); families live one layer down in groups, LazyVim
style: Top bar `␣T`, Draft `␣D`, Markup `␣M`, Window `␣W`, Interface `␣U`.
A group opens only when one of its commands can run in the current focus
(Markup needs an editor, Interface the Prompt pane; Top bar and Draft run from
anywhere) and otherwise leaves its key to the root meaning. New families
become groups instead of taking root letters. Free root letters: `B E G I O
R V X Y`.

Undo/redo stay with the editor (`u`/`Ctrl-R`, `Mod+Z`/`Mod+Shift+Z`).
Numbered session slots (`Alt/Option+1…0`) are retired: labels replace them,
so Option+digit types its character again in text fields.

### Product letters ignore case; Vim motions do not

Bare contextual product shortcuts (`F` Follow, `Z` Reading, `V`
History/Explore, top-bar `R`/`U`/`A`/`L`/`C`/`X`, run-config mnemonics) match the physical letter with
or without Shift. They are not a second Shift-modified command. Vim regions
keep case: `g`/`gg` versus `G`, and list/transcript `j`/`k`/`h`/`l`. Modified
chords (`Mod+Enter`, `Shift+J`) still require their exact Shift state.

### One truthful shortcut state machine

Every live shortcut slot uses `ShortcutKeycap` and exactly one state:

- `inactive`: the chord will not execute now because its focus scope does not
  own input or its action is currently unavailable;
- `available`: pressing the displayed chord now executes the advertised action;
- `active`: a transient command owner is engaged, such as an armed prefix, an
  open overlay's launcher, or another pending keyboard mode.

`active` is not a synonym for selected, pressed, current, or focused. Tabs,
segmented choices, selected rows, and toggles communicate those states through
their own MUI semantics while their usable shortcut remains `available`.
Business-disabled controls keep their native disabled treatment and explanation;
their keycap becomes `inactive` because the chord cannot execute. The DOM must
expose the same truth through `data-shortcut-state`; opacity or color alone is
never the state model.

Global shortcuts are available without first focusing a region and stay visible
as quiet embedded slots. Context shortcuts become available only when their
owning region or modal owns keyboard input. Opening an overlay may make its
launcher `active`, but commands underneath that exclusive overlay become
inactive even if they were previously focused.

Availability must come from the same focus and business predicates used by the
dispatcher; visibility, hover, and selection are not substitutes. A live slot
must update on focus transfer, editor entry, pending async work, and overlay
ownership in the same render that changes command execution. Shortcut-guide
tables are explicitly reference material (`data-shortcut-reference`), not live
slots, and must not be copied into an action surface as an availability claim.

### Activation, confirmation, and escape

- `Enter` opens, selects, or activates the focused non-destructive item.
- `Mod+S` saves Queue/Draft edits. `Mod+Enter` sends/queues a new prompt or confirms a consequential action. A
  confirmation must never also accept plain Enter.
- `Esc` unwinds exactly one innermost transient state: pending chord, Vim Insert
  or reorder mode, popover, modal, then product mode. It does not skip levels.
- Leaving a dirty transactional edit may ask for discard confirmation, but only
  after editor-owned modes have returned to plain Normal. Confirm discard still
  uses `Mod+Enter`; `Esc` keeps editing.

### Vim motion and focus ownership

Within a keyboard workbench, `J/K` moves vertically between fields or items and
`H/L` moves horizontally between choices or adjacent panes. Reader-like
surfaces add `Ctrl-D/U` for half-page motion, `Ctrl-F/B` for full-page motion,
and `gg/G` for the two ends. A visible shortcut bar is a live legend for these
surface-owned motions. Text inputs and CodeMirror own unmodified input; active
IME composition owns every key before workspace navigation. The one exception
is the explicit workspace prefix, which works from native inputs and all
CodeMirror Vim modes after composition has ended.
Chrome shortcuts stay with Chrome whenever Cowboy has no matching command. Do
not install a blanket keydown shield for Find, Open, Downloads, address-bar,
tab, window, reload, zoom, or DevTools actions. The narrow intentional
overrides are specified in the collision policy below.
While an editable field owns focus, the workbench motions become inactive and
the field/search owner may become active. `Esc` only unwinds its innermost
editor or overlay state; it never arms workspace navigation. Returning to Prompt
through the workspace prefix preserves Insert/Visual mode, selection, and caret.

### Key ownership and input methods

New Desktop key handlers classify a keydown through `desktopKeyIntent`
(`commands/keyIntent.ts`) before binding anything. It returns exactly one owner:

- `ime`: `isComposing`, the CM6 composition store, or a native-input
  composition (tracked document-wide from `compositionstart` until 50 ms after
  `compositionend`) owns the key. Do nothing and never `preventDefault()`; for
  `Esc`, only stop propagation so the modal underneath does not close.
- `text`: an unmodified key aimed at a native field or editor. An IME marker
  (`Process`, 229, dead/text-service keys) there is `ime`, not text.
- `command`: a bindable key with its physical identity (`j`, `G`, `1`, `[`,
  `Escape`). Modified chords become commands once no composition exists,
  even when an idle CJK source labels them Process/229. Non-editable chrome
  and the Vim Normal sink resolve physical letters and digits.

Do not compare `event.key` letters or digits in Desktop handlers or add another
local `isComposing`/229 check; extend the classifier and its tests instead.
Older handlers (`desktopImeOwnsKey`, `workspaceCommandKey`) retain their
established behavior and should move onto it when touched.

### Create

Create follows the browser-Vim layering of Vimium and qutebrowser. It opens in
the title field (Insert). `Esc` or `Ctrl-[` leaves a text field for the
selected type tab (Normal) instead of closing; a composing IME keeps `Esc`.
On the tablist, `H/L` (or arrows) moves between Session, Draft and Folder
without leaving it, `I`, `J`, `↓` or `Enter` returns to the title, and
`Esc` closes. `Mod+Enter` creates from either layer. A direct pick of any
control (Session, Draft, Folder, Project, …) is the dialog leader
(`Cmd/Alt+K` or `Space`, then its letter); the tabs carry no digits. The hint
row under the tablist names only the current layer's keys. Pointer activation still selects
and focuses the title in one step. The Cancel `Esc` keycap is inactive while a
text field owns `Esc`.

### Shortcut slots and bars

Every keyboard-capable action has one discoverable slot. Fixed controls embed
the slot in the component; contextual item actions anchor it to that action;
navigation-rich workspaces and modals use the shared `DesktopShortcutBar` at
their bottom edge. A simple two-action confirmation keeps `Esc` and `Mod+Enter`
beside the buttons instead of adding a redundant bar. Dense repeated lists may
hide item action slots until that item owns focus, but their header/prefix slots
remain present with truthful inactive states.

A shortcut bar describes only bindings implemented by the surface that owns
it. Do not advertise browser defaults, reference-only examples, or a command
handled by a layer underneath the current modal. Its groups should follow task
order—Navigate, Page, Jump, Commit/Close—and may scroll horizontally instead of
wrapping into a second toolbar.

### Sequential chords

A scoped sequence such as `'` then a row label has a three-state transition:

1. outside its scope, prefix and continuations are `inactive`;
2. in scope, the prefix is `available` and continuations remain `inactive`;
3. after the prefix, it becomes `active` and valid continuations become
   `available` for its documented timeout.

A valid continuation executes and clears the sequence. `Esc`, timeout, focus or
mode change, and any unrelated unmodified continuation cancel it; the unrelated
key is consumed so it cannot trigger a row action accidentally. A modified
global chord cancels the sequence and continues normally. Auto-repeat must not
turn one held prefix into a completed double-key command.

The leader is the separate global sequence described above. It does not time
out, and it never escapes an IME composition, modal, menu, or other exclusive
shortcut scope. Releasing the `Cmd/Alt` prefix modifier before the
continuation is optional.

## Navigation

- Leader then `S/P/C`: focus Sessions, Prompt or Conversation; `␣TT` focuses
  the Top Bar.
- Leader then `L/Q/D`: focus Plan, Queue, or Drafts.
- Leader then `N` or `,`: create, or open Settings. The Window group
  `␣WW`/`␣WR` cycles visible regions or enters Resize mode.
- `␣UE` (Prompt pane): toggle the composer between live
  preview and Source mode. Obsidian binds this to `Mod+E`, which both collision
  audits reject — Chrome owns it for the address bar and desktop apps for a
  common editor action — so the toggle lives in the Interface group and keeps
  the `E` mnemonic. It works from Vim Insert, Normal, and Visual
  because the preference is global rather than an edit on the document.
- `␣W[` / `␣W]` / `␣W\`: collapse or expand Sessions, Prompt, or
  Conversation. The three adjacent keys sit in the same left-to-right
  order as the panes. See [Pane collapse](#pane-collapse).
- `␣␣` then a label switches session; ``␣` `` returns to the previous one.
- `Mod+Enter` sends or queues, `Mod+S` saves a draft, `Mod+.` stops the current
  turn, `Mod+Shift+P` opens Command Palette, and `Mod+/` opens shortcut help.
- `Alt/Option+Enter` force-pushes a prompt. On macOS, no other product action
  reserves an Option letter; on Windows/Linux, `Alt+K` is additionally the
  workspace prefix. Slash, references, attachments, scheduling, queue priority,
  expand, and More remain visible in the UI and searchable in Command Palette.
- In the main Composer, leader then `/`, `F`, `A`, `H`, `J` opens slash
  commands, file references, attachments, scheduling or queue priority; `Z`
  zooms the editor. Formatting is the Markup group: `␣MB` bold, `␣MI` italic,
  `␣MX` inline code, `␣MU` link, `␣MO` bulleted list, `␣MM` More. These are
  scoped to `prompt.composer`; `␣UE` Source mode retains its whole-Prompt scope.
  Every toolbar button carries its `␣` slot, which lights while the leader is
  armed; tooltips and the Command Palette show the same keycap. A claimed direct chord stops
  propagation to editor fallbacks. In particular, `Alt+Enter` must never also
  save a draft, even when Force push is unavailable.
- In Resize mode, `H/L` moves the selected split.
- `j/k`, `gg`, `G`: item navigation outside text-editing controls. Conversation
  is a reader rather than an item list, so the same keys scroll by line or jump
  to the oldest/latest output there.
- In Sessions, the filled row is the currently open session only while
  Sessions owns focus; a distinct accent cursor shows the row selected by
  `j/k` for the next `l`/Enter action. Switching to a Conversation tab
  (History/Explore/Reading) must un-highlight the session row so the tab is
  the only selected chrome.
- `Enter`: default item action. In Sessions, `l` and `Enter` open the selected
  session and move focus to its Prompt editor; entering the Sessions region
  always starts on the currently open session.
- Sessions is a folder tree (`docs/sessions-folders.md`); folder rows are
  items like session rows. On a folder row `l` expands and `Enter` toggles.
  `h` is the tree's left motion: it collapses an expanded folder, moves to the
  parent from a collapsed one, collapses every folder from a top-level one,
  and moves from a session row to its folder. `s` opens the row's actions
  (the session Settings modal, or the folder actions modal), `m` opens Move
  to…, `n` creates a folder (inside a folder, beside a session), and `i`
  renames. All are region-scoped bare letters resolved from physical codes.
- Sessions use `o` to enter or leave Order reorder mode. While pinned, `j/k`
  moves the selected row instead of moving selection: a session that crosses a
  folder header files into that folder, a folder moves among its siblings, and
  `Esc` releases the mode. The trailing three-dot menu retains secondary
  actions such as Rename, Move to folder and Delete.
- `␣␣` labels sessions by their flat displayed order (the one the Mobile
  drawer also renders), never by the folded view; labels show on visible rows
  and in the switcher panel. Rows show no keycap at rest.
- `i`: edit the item when it exposes an edit action.
- `Esc`: close the current transient layer or leave editor Insert mode.
Text inputs and CodeMirror retain their own Vim/IME semantics. Workspace list
navigation must never intercept unmodified keys while a text-editing target
owns focus.
Workspace Vim letters are resolved from physical `Key*` codes so a non-Latin
macOS input source cannot turn `j/k`, `gg/G`, `l`, or `i` into marked text.

## Conversation reader

The Conversation header owns the visible `Following`/`Follow` control. It is
session state, not a global top-bar action. While Conversation is focused:

- Page View's question navigator is transient rather than a permanent column:
  `p` opens it as a modal without resizing or covering one side of the reader,
  presents newest pages first, and uses Vim list navigation: `j/k` moves
  the cursor, `l`/`Enter` opens, `h`/`Escape` closes, `Ctrl-d/u` and
  `Ctrl-f/b` scroll, `gg/G` jumps to the first/last row, and `/` focuses
  search. The global status line replaces ordinary reader shortcuts with
  Page-specific shortcuts; the Conversation pane header does not duplicate
  them.
- `v` toggles the Conversation projection between History and Explore.
- `j/k` scroll down/up by one reading line;
- `Ctrl-d/u` scroll down/up by half a page;
- `Ctrl-f/b` scroll down/up by one page;
- `gg/G` jump to the oldest/latest output;
- `Shift-f` toggles automatic following. Bare `f` never opens a page-wide
  target overlay; Prompt Vim retains native `f<char>`, and workspace actions
  remain discoverable through visible contextual shortcuts and Command Palette.
- `Tab/Shift-Tab` selects the next/previous expandable transcript widget;
- `h/l` closes/opens the selected widget, and `Enter` toggles it. With no
  selection these keys target the widget nearest the viewport centre;
- `a/r` allows/rejects a pending tool permission. Cowboy chooses the least
  persistent matching provider option, preferring `once` over `always`.

Any navigation away from the latest output pauses following. `G`, or enabling
Following again, returns to the latest output. The status line exposes this
complete map while the reader owns focus, so the bindings remain discoverable
without permanent badges in the transcript header.

## Desktop bundle recovery

Desktop installs a small pre-module recovery guard in `index.html`. It exists
before React so an obsolete hashed entry or lazy chunk can recover after the
server atomically switches bundles. On a module-load failure it waits for
`/version`, asks the Service Worker to update, then reloads with a cache-busting
query. Three failures within one minute stop automatic retries and present a
manual retry action. Mobile retains its separate PWA recovery path.

## Prompt workspace sizing

Plan, Queue and Drafts are independent Desktop regions, not children of
Mobile's shared `40vh` touch scroller. Their headers remain visible as stable
jump targets. Focusing one expands its own bounded list and releases the other
auxiliary lists; focusing Composer releases all three lists so the writing
canvas receives the column. A jump into a manually collapsed region expands it
first and focuses its first item. Editing a queued prompt or draft keeps that
region expanded, focuses the row editor after it mounts, and scrolls the row to
the center without smooth-scroll latency.

Mobile retains its shared capped scroller and fullscreen-first row editing. It
must not load or emulate this focus-driven sizing contract.

## Pane collapse

Sessions, Prompt and Conversation can each be collapsed without changing
product mode. The state is one global, persisted layout preference
(`cowboy:desktop-collapsed-panes`), never per Session, and Mobile never reads
it.

- **Commands.** `␣W[` / `␣W]` / `␣W\` toggles Sessions /
  Prompt / Conversation. The Command Palette also lists each toggle and
  Expand All Panes. Every pane header has a collapse control (chevron plus
  its continuation keycap) at its trailing edge.
- **Keycaps.** The `[` `]` `\` keycaps on headers and rails are sequence
  continuations, so they follow the sequential-chord law: inactive at rest,
  available while the prefix is armed. Pressing the prefix lights all three
  across the screen, left to right. The status line lists them as Fold panes.
- **Work-pane invariant.** Prompt and Conversation never collapse together.
  Collapsing the last visible one swaps them instead, so the chord always
  does something visible. Stored layouts are normalized the same way.
- **Collapsed presentation.** Prompt and Conversation fold into a 36 px rail
  on their own edge: chevron, keycap, vertical pane name, and the signals the
  user may be waiting on (Prompt: queued/draft counts; Conversation: the
  Session's live status). The whole rail is one restore target. Sessions
  folds into a 56 px rail that shows the user's folder structure, because a
  column that narrow cannot show titles: expand control, New Session, then
  one labelled entry per top-level folder plus Unfiled (just Sessions when
  there are no folders). Each entry carries one actionable badge (needs
  attention in amber, else working in green; ready and dormant never badge)
  and the group holding the open Session gets the edge pill. Clicking an
  entry opens a menu with real titles, subfolder headings, status,
  and Show all sessions (`␣␣` still switches directly); the rail ignores this
  device's folder folds. The rail head aligns with the 44 px top bar;
  in an installed window-controls-overlay PWA the head becomes a drag region
  and the expand control moves below it.
- **Mounting.** Collapsed panes stay mounted and are only removed from
  layout, so the Composer keeps its draft, undo history, Vim mode and IME
  state, Conversation keeps streaming, and the Sessions list keeps owning
  slot switching and folder state. A Prompt that inherits the Conversation's
  width uses the full available writing canvas.
- **Split.** Beside the full Sessions list, Prompt keeps its persisted pixel
  width. With Sessions collapsed, Prompt and Conversation split the
  workspace evenly by default; dragging or keyboard-resizing that splitter
  stores a separate ratio (`cowboy:desktop-prompt-ratio-sessions-collapsed`,
  25–75%), so neither layout's resize leaks into the other. Both still honour
  the 360 px Prompt and 520 px Conversation floors.
- **Focus.** Collapsing the focused pane moves focus to the remaining work
  pane; restoring one by its command focuses it. Explicit jumps (prefix
  `P/C`, Plan/Queue/Drafts, region focus) restore a collapsed Prompt or
  Conversation first. Collapsed Sessions is different because its rail is
  still a navigable pane: prefix `S` focuses the rail (`sessions.rail`) on
  the folder holding the open Session and keeps the layout; only `[`
  unfolds the list. Region cycling and Resize mode skip collapsed panes and
  their splitters. `␣␣` never unfolds Sessions; it lands in Prompt (or
  Conversation when Prompt is collapsed).
- **Rail keys.** In `sessions.rail`: `J/K`, `gg`/`G` move between folders;
  `L` or `Enter` opens the focused folder's menu; `'` labels the folders
  for a direct jump. The menu opens on the open Session (or its first) and
  owns the keyboard: `J/K` or arrows move, `L`/`Enter` opens the Session and
  focuses Prompt (Conversation when Prompt is collapsed), `H`/`Esc` returns
  to the same folder in the rail, and its footer lists those keys. MUI's
  first-letter type-ahead is suppressed for `J/K/H/L`. The status line shows
  the rail map, Switch (`␣␣`) and Expand list (`␣[`).
- **Pointer.** Dragging a splitter more than 96 px past a pane's minimum
  previews the collapse by dimming that pane and collapses it on release; the
  stored width is kept for the restore. Panes switch instantly: animating the
  width would reflow the Transcript and CodeMirror on every frame.
- **Compact Desktop.** Below 1100 px Sessions already lives in its drawer, so
  `[` opens or closes that drawer and the wide-layout preference is left
  untouched.

## Commands and help

Commands may declare pane `contexts` and exact `regions`. The Command Palette
searches every registered command; the status line lists only actions available
in the current region.

The Desktop-only shortcut dialog opens from the status-line shortcut slot or
`Mod+/`; `Mod+Shift+P` opens the all-command palette. Workspace destinations use
the single platform prefix documented above. No global product action uses a
bare letter, colon, question mark, comma, or backslash.

Desktop does not reserve bare `F` for a page-wide target overlay. Focus moves
through the workspace prefix, native focus, contextual Vim motions, and the
Command Palette; Conversation keeps `F` for Following.

Shortcut hints implement the core state machine above with three discovery
levels:

1. global shortcuts are always visible but quiet (leader slots and direct
   semantic chords) because they work without first focusing a region;
2. contextual shortcuts float over their action only while the owning region is
   focused (Prompt subregions and list item actions), so they add no layout
   width and disappear when attention moves elsewhere;
3. modal actions use their surface-owned shortcut bar, while simple
   confirmations show the real confirmation/dismissal chord next to the button.

Top Bar controls show their `␣T` group slot, available whenever the control
itself is enabled. Other embedded contextual shortcuts are the persistent
exception to visibility gating: they keep their one-key badge visible for
discovery, but the shared keycap
primitive must render it as `data-shortcut-state="inactive"` whenever its
owning region is not focused. Once the region owns focus it becomes
`data-shortcut-state="available"` and gains the normal accent treatment. Do not
approximate these states with component-local opacity or colors; all persistent
contextual badges must use `ShortcutKeycap` availability so enabled and inactive
semantics remain identical across Desktop.

Queue and Draft headers show their `'` label trigger; rows carry no ordinal.
Outside the list the trigger is inactive; while the list owns Normal-mode
focus it is available. Pressing it makes it active and paints a letter on
every visible row (home row first, top to bottom). A label focuses that exact
row and clears every label. Cancellation follows the shared sequential-chord
law, so a modified global command such as `Cmd/Alt+K` still arms the leader
immediately. The same `'` works in Sessions and the collapsed Sessions rail.

Inside a modal, `Space` (on a non-text control) or `Cmd/Alt+K` (anywhere)
arms the modal's own leader. Every operable control of the topmost modal gets
a stable mnemonic letter (an explicit `data-leader-key` first, then word
initials, then any free letter), painted on the control and listed in the
which-key panel. The next key activates it the way a pointer would: fields
focus, Selects open, buttons and tabs click. Menus, listboxes and popovers
keep their own keys and never get labels.

List-row action hints are item-scoped: focusing Queue or Drafts reveals hints
only on the current `[data-desktop-item]`, never on every row merely because the
region owns focus. A badge must describe an actual binding; `L`/`Enter` belongs
on the focused row's Edit action, while unbound pointer actions stay unlabelled.

Never invent a hint for an action that is not wired. Contextual hints anchor to
the bottom-right of their target. Prefer a convenient bare key only inside a
clearly owned, non-editor context; then a standard semantic or Vim chord; then
the stable workspace prefix. Global bare product letters are forbidden. Keep
native semantic chords such as `Mod+S`, and put secondary Cowboy-specific
actions in the searchable command palette. Shared modal shells may use the same
visual primitive, but must hide it on the touch product.

### Browser and operating-system collision policy

Not conflicting with Chrome is a core Desktop requirement, in ordinary tabs
and installed PWAs. Every registered command must pass both checked-in audits:
`chromeShortcutPolicy.ts` and `macShortcutPolicy.ts`.

- Chrome tab, window, address-bar, history, download, bookmark, navigation,
  reload, print, find, zoom, and DevTools chords are unavailable to Cowboy.
  Examples include `Ctrl/Cmd+N/T/W/L/E/P/R/F/J/H/D/1…0/Tab`, Windows/Linux
  `Ctrl+K`, and `Alt+Left/Right`. Do not rely on `preventDefault()` to make one
  usable.
- A Chrome chord may be overridden only when the Cowboy action has the same
  established semantic or is a standard Vim reader motion in an exclusively
  owned context. The current browser exceptions are `Ctrl/Cmd+S` Save Draft and
  Conversation/Reading `Ctrl+D/U/F/B`; macOS additionally treats `Cmd+.` Stop
  as a matching native-style semantic action. Additions require an explicit
  policy entry, tests, visible help, and an update to this section.
- When Cowboy has no command, do not swallow the event. Chrome Find, Open,
  Downloads, view-source, and other browser behavior must continue to work.
- macOS destructive and system chords such as `Cmd+Q/W/H/M/Tab/Space`, input
  source chords, screenshots, and Option dead keys stay reserved. Bare `Q` is
  also reserved because it can become `Cmd+Q` while Command is being released.
- Workspace navigation uses `Cmd+K` on macOS. Chrome does not claim that chord
  there; Cowboy deliberately overrides macOS's common Add Link semantic for the
  stable workspace prefix. Windows/Linux use `Alt+K` because Chrome owns
  `Ctrl+K` for omnibox search.
- Prefix navigation works in editors, but active IME composition and exclusive
  modal/menu scopes always win. A direct shortcut cannot overlap a global and a
  contextual command; contextual reuse is allowed only across provably disjoint
  scopes. Each prefix continuation has one stable command meaning.
- Space is not a browser or system chord; Cowboy takes it only where no
  text, toggle or IME owns it (see Leader), giving up page scrolling by Space
  in the reader in favour of `Ctrl-D/F`.
- Pane-collapse continuations `[` `]` `\` are free under the prefix. When the
  prefix modifier is still held, Windows/Linux `Alt+[ ] \` are not Chrome
  chords; macOS `Cmd+[` / `Cmd+]` are Chrome Back/Forward, which Chromium does
  not reserve, so the claimed continuation's `preventDefault()` keeps them in
  Cowboy. Releasing Command before the continuation avoids the question.

For every new shortcut, update the central shortcut constants, both collision
audits where relevant, policy tests, visible hints, and this guide. Acceptance
must include real Chrome on macOS and Windows/Linux; extensions and user-level
OS remaps are outside the static guarantee. Reference inventories:
[Chrome keyboard shortcuts](https://support.google.com/chrome/answer/157179)
and [Apple Mac keyboard shortcuts](https://support.apple.com/en-us/102650).

An expanded Queue or Draft region starts on its first row. `j/k` moves the row
selection, while `l` or `Enter` opens the selected message for editing. The
inline queued/draft editor uses `Mod+S` to save and `Esc` to open the
discard confirmation; plain `Enter` remains a newline. Slash, reference,
attachment, and expand remain visible actions and Command Palette entries but
do not reserve Option/Alt letters. Mobile renders neither bindings nor hints.

## Visual hierarchy

The main Prompt editor is a flat pane surface with no nested card outline,
focus halo or outer reading-width gutter. The pane header and status line own
focus indication. Its bottom editing and delivery groups stay outside the
editor scrollport. Container queries use the Prompt's actual width and the root
font size to disclose quick formatting and labels; the formatting menu retains
all commands. Send/Queue stays labelled at the right edge, while Draft,
Schedule, Run next and Force push remain direct controls and may wrap when
space is limited. Empty, preparing, busy, paused, disconnected, pending-submit
and resumed states change labels/availability without replacing the editor.
Quota exhaustion is an inline notice across the Desktop pane; Mobile keeps its
existing card and keyboard geometry.

Only the focused region gets the subtle accent rail/background. The focused
item uses the MUI selected/focus-visible treatment. Avoid simultaneous heavy
outlines at pane, region and item levels.

Desktop geometry has two levels. First-level interactive surfaces—including
Top Bar controls, Session rows, Queue rows, and Draft rows—always use
`DESKTOP_SURFACE_RADIUS`. Chips, shortcut groups, tiles, and other content
nested inside those surfaces use `DESKTOP_INSET_RADIUS`. Never apply the inset
radius to a whole selectable row; changing geometry must happen in the shared
Desktop primitive rather than in a component-local override.

Session rows deliberately separate two states while Sessions owns focus: the
currently open session uses a tinted fill, while the transient keyboard cursor
uses a crisp outline. When both states coincide, both signals remain visible.
Do not add a heavy leading rail or reuse the same fill treatment for current
state and J/K focus. When Conversation or Prompt owns focus, session rows stay
unmarked so the tab or editor is the only highlighted selection; the live
session remains identifiable from its status mark and the open transcript.

Desktop product modes are separate command domains. Agent is the default mode;
`Z` enters Reading only while Conversation owns focus. Reading covers the Agent
chrome, `Esc` returns to Agent, `V` switches History/Page, `P` toggles one shared
question directory, and `F` follows the live edge. The directory is available in
both projections: History selection locates the question root in the continuous
transcript, while Page selection opens that isolated question. Its focused Vim
list owns `J/K`, `gg/G`, `Ctrl-D/U`, `Ctrl-F/B`, `L`/Enter and `H`; Reading-level
`Esc/P/V/F` remain available. Following from an older Page returns to the latest
question before resuming live output. Agent pane/session/queue commands must not
leak into Reading. Future Code mode uses the same product-mode boundary rather
than adding another Agent overlay.

The Agent Conversation header exposes Reading as its own embedded action between
the History/Explore projection switch and Following. Reading is not a third
projection: entering it preserves the selected projection and changes only the
product mode. Its visible `Z` slot is inactive outside Conversation and available
while Conversation owns focus, matching the registered command exactly.

Workspace prefix then `P` always enters `prompt.composer`, even when Sessions,
Conversation, Plan, Queue, or Draft currently owns focus. It restores the
preserved Insert/Visual mode, selection, and caret rather than forcing Normal.
Prefix then `L` enters Plan. When no Plan exists the continuation remains
reserved and disabled rather than acquiring a transient second meaning.

The Composer is the exception: its caret and outlined editing canvas already
communicate focus, so `prompt.composer` must not receive the generic region
background, accent rail, or focus ring. When Conversation, Sessions, or the
top bar is the highlighted workspace region, the hidden Prompt Vim sink must
not consume typed keys or IME input; those keys belong to the highlighted
chrome and must not pop Prompt into Insert.

Workspace prefix then `W` cycles visible workspace regions. Prompt Plan, Queue, and Draft are
auxiliary panels rather than Vim windows; enter them through their dedicated
commands, so region cycling never collapses or selects them as an intermediate
stop.

Workspace prefix then `R` selects the nearest visible vertical boundary and enters layout
Resize mode without moving it: Sessions / Prompt from Sessions, Prompt /
Conversation from either work pane, or Page index / Page in Reading mode.
The selected bar uses the shared accent and keycap language; `H/L` moves it by
16px, `Shift-H/L` moves it by 48px, `Tab` cycles visible bars, and `Esc` or
`Enter` returns to the previously focused region.
Resize mode is exclusive, so unrelated bare keys never leak into lists,
transcript widgets, or destructive actions. Pointer dragging keeps working and
selecting a bar with the pointer enters the same visible state.

Queue and Draft use the same list contract as Sessions: `J/K` selects, `gg` and
`G` jump to the ends, and `'` then a label jumps to any visible row.
`L`/`Enter` opens the
selected message editor. `O` pins Order reorder mode
so `J/K` moves the message and `Esc` releases it; `Shift+J/K` moves it
directly. The rows are flat lines of the Prompt with a primary edge on the
current row; the pointer grip is a slim edge handle revealed on hover or focus
and carries the `O` hint. Row actions are `S` send, `R` return to drafts, `T`
schedule, `M` move to another Session, `D` move to independent Drafts and `X`
remove. Inside the editor, `Mod+S` saves and `Esc` cancels, with both
returning focus to the originating list row.

### Draft document

Independent Draft documents share the Prompt editor region and command host.
They autosave; `Mod+S` flushes local persistence without sending. The leader
exposes the same formatting, attachment and Source commands as Sessions.

The document itself is a `␣D` group, available from any focus while a Draft is
open (it shadows `␣D` Focus Drafts, which has no list on that page):
`␣DR` rename, `␣DV` copy to Session drafts (source retained), `␣DH` recovery
history, `␣DE` export Markdown, `␣DW` readable width. Each control shows its
`␣D…` slot; Desktop draws the document actions once, in the bottom bar.

The title behaves as the document's first line, as Obsidian's inline title:
`␣DR` focuses it with the text selected for replacement; `↑` in Insert or a
plain Vim Normal `k` on the body's first line enters it at the end (a pending
Vim command such as `dk` keeps its key); `Enter`, `↓` or `Tab` returns to
the start of the body and `Esc` returns to where the body caret was. Every
action is also in the Command Palette.
