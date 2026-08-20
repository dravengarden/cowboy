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

## Core interaction laws

These rules are the canonical Desktop-mode primitive contract. New controls,
modals, product modes, and shortcut hints must reuse them rather than creating
component-local keyboard semantics.

### Product letters ignore case; Vim motions do not

Bare product shortcuts (`F` Follow, `Z` Reading, `V` History/Explore, top-bar
`R`/`U`/`L`/`C`/`X`/`S`, run-config mnemonics) match the physical letter with
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
- `Mod+Enter` commits a mutating edit or confirms a consequential action. A
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
surface-owned motions. Text inputs, CodeMirror, native selection, and active IME
composition always own their native keys before workspace navigation.
Chrome shortcuts stay with Chrome whenever Cowboy has no matching command. Do
not install a blanket keydown shield for Find, Open, Downloads, address-bar,
tab, window, reload, zoom, or DevTools actions. The narrow intentional
overrides are specified in the collision policy below.
While an editable field owns focus, the workbench motions become inactive and
the field/search owner may become active. `Esc` first returns focus to the
workbench; in the main Composer a second `Esc` arms the one-shot workspace map.

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

A sequence such as `G` then `1…0` has a three-state transition:

1. outside its scope, prefix and continuations are `inactive`;
2. in scope, the prefix is `available` and continuations remain `inactive`;
3. after the prefix, it becomes `active` and valid continuations become
   `available` for 1.2 seconds.

A valid continuation executes and clears the sequence. `Esc`, timeout, focus or
mode change, and any unrelated unmodified continuation cancel it; the unrelated
key is consumed so it cannot trigger a row action accidentally. A modified
global chord cancels the sequence and continues normally. Auto-repeat must not
turn one held prefix into a completed double-key command.

## Navigation

- `S/E/P/C/T`: focus Sessions, Editor, Plan, Conversation, or Top Bar.
- `Y/D`: focus Queue or Drafts; `W` cycles visible regions.
- `Alt/Option+1…0`: switch to one of the first ten Sessions globally.
- `\`: enter Resize mode on the nearest vertical split. `H/L` then moves it.
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
- Sessions use `o` to enter or leave Order reorder mode. While pinned, `j/k`
  moves the selected session instead of moving selection, and `Esc` releases
  the mode. `h` opens the selected session's Settings directly; the trailing
  three-dot menu retains secondary actions such as Rename and Delete.
- `i`: edit the item when it exposes an edit action.
- `Esc`: close the current transient layer or leave editor Insert mode.
- `Alt-b/p/c/t`: jump directly to Sessions, Prompt, Conversation or Top Bar.
- `Mod-p` focuses Plan from anywhere and `Mod-i` focuses Message the agent in
  Vim Normal mode from anywhere. Each global chord has one stable meaning;
  bare `p/P` always remains native Vim paste. `Mod-y` and `Mod-d` jump to Queue
  and Drafts after Prompt owns focus.
- `Mod-y` is also the Queue disclosure action: from another Prompt region it
  expands and focuses Queue; from Queue it collapses the panel and returns to
  the Composer in Normal mode.
- `Mod-1…0`: jump to one of the first ten visible items in the focused region.
  In Sessions this switches the active session immediately while retaining
  focus in the rail, so the next `j/k` continues from the selected row.
- `Mod-j/k`: reorder the focused Queue or Draft item. Sessions use the more
  deliberate Order mode above so ordinary navigation and reordering cannot be
  confused.
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

## Commands and help

Commands may declare pane `contexts` and exact `regions`. The Command Palette
searches every registered command; the status line lists only actions available
in the current region.

The Desktop-only shortcut dialog opens from the status-line `? Shortcuts`
entry or bare `?`. Bare `:` opens the all-command palette. Outside an editor,
workspace commands are direct: `S` Sessions, `E` Editor, `P` Plan, `C`
Conversation, `T` Top Bar, `Y` Queue, `D` Drafts, `N` New Session, `W` next
region, `,` Settings, and `\` Resize. In the main Composer, the first `Esc`
returns to Normal and a second `Esc` arms the same one-key workspace map for
1.2 seconds.

Desktop does not reserve bare `F` for a page-wide target overlay. Focus moves
through the direct workspace map, native focus, contextual Vim motions, and the
Command Palette; Conversation keeps `F` for Following.

Shortcut hints implement the core state machine above with three discovery
levels:

1. global shortcuts are always visible but quiet (`S/E/P/C/T`, `N`, `,`,
   and `Alt/Option+1…0`) because they work without first focusing a region;
2. contextual shortcuts float over their action only while the owning region is
   focused (Prompt subregions and list item actions), so they add no layout
   width and disappear when attention moves elsewhere;
3. modal actions use their surface-owned shortcut bar, while simple
   confirmations show the real confirmation/dismissal chord next to the button.

Embedded contextual shortcuts are the persistent exception to visibility
gating. Controls such as Top Bar Run Configuration, Usage, Compact, Clear, and
Stop keep their one-key badge visible for discovery, but the shared keycap
primitive must render it as `data-shortcut-state="inactive"` whenever its
owning region is not focused. Once the region owns focus it becomes
`data-shortcut-state="available"` and gains the normal accent treatment. Do not
approximate these states with component-local opacity or colors; all persistent
contextual badges must use `ShortcutKeycap` availability so enabled and inactive
semantics remain identical across Desktop.

Queue and Draft headers show their sequential `G` prefix and their first ten
visible rows show `1` through `9`, then `0`. Outside the list all are inactive;
while the list owns Normal-mode focus, `G` is available and the numeric slots
remain inactive. Pressing `G` makes the prefix active and numeric slots
available. A valid slot focuses that exact row and a second `G` focuses the
first row. Cancellation follows the shared sequential-chord law, so a modified
global command such as `Alt/Option+1` still switches sessions immediately.

List-row action hints are item-scoped: focusing Queue or Drafts reveals hints
only on the current `[data-desktop-item]`, never on every row merely because the
region owns focus. A badge must describe an actual binding; `L`/`Enter` belongs
on the focused row's Edit action, while unbound pointer actions stay unlabelled.

Never invent a hint for an action that is not wired. Contextual hints anchor to
the bottom-right of their target. Prefer a convenient bare contextual key; then
a standard semantic or Vim chord; then one browser-safe modifier. Use a
sequence only when those choices are exhausted. Do not add a Space leader.
Keep native semantic chords such as `Mod+S`, and put secondary Cowboy-specific
actions in the searchable command palette. Shared modal shells may use the
same visual primitive, but must hide it on the touch product.

### Browser and operating-system collision policy

Not conflicting with Chrome is a core Desktop requirement, in ordinary tabs
and installed PWAs. Every registered command must pass both checked-in audits:
`chromeShortcutPolicy.ts` and `macShortcutPolicy.ts`.

- Chrome tab, window, address-bar, history, download, bookmark, navigation,
  reload, print, find, zoom, and DevTools chords are unavailable to Cowboy.
  Examples include `Ctrl/Cmd+N/T/W/L/K/E/P/R/F/J/H/D/1…0/Tab` and
  `Alt+Left/Right`. Do not rely on `preventDefault()` to make one usable.
- A Chrome chord may be overridden only when the Cowboy action has the same
  established semantic or is a standard Vim reader motion in an exclusively
  owned context. The current exceptions are `Ctrl/Cmd+S` Save Draft and
  Conversation/Reading `Ctrl+D/U/F/B`. Additions require an explicit policy
  entry, tests, visible help, and an update to this section.
- When Cowboy has no command, do not swallow the event. Chrome Find, Open,
  Downloads, view-source, and other browser behavior must continue to work.
- macOS destructive and system chords such as `Cmd+Q/W/H/M/Tab/Space`, input
  source chords, screenshots, and Option dead keys stay reserved. Bare `Q` is
  also reserved because it can become `Cmd+Q` while Command is being released.
- Workspace navigation uses the bare map above outside editors and the
  second-`Esc` one-shot map inside Vim. IME composition and editable Insert
  mode always win.
- Session slots are the deliberate cross-platform modifier exception:
  `Alt+1…0` on Windows/Linux and `Option+1…0` on macOS. All ten slots are
  reserved even when a slot is empty, so the key never changes meaning with
  session count.

For every new shortcut, update the central shortcut constants, both collision
audits where relevant, policy tests, visible hints, and this guide. Acceptance
must include real Chrome on macOS and Windows/Linux; extensions and user-level
OS remaps are outside the static guarantee. Reference inventories:
[Chrome keyboard shortcuts](https://support.google.com/chrome/answer/157179)
and [Apple Mac keyboard shortcuts](https://support.apple.com/en-us/102650).

An expanded Queue or Draft region starts on its first row. `j/k` moves the row
selection, while `l` or `Enter` opens the selected message for editing. The
inline queued/draft editor follows the same discoverable toolbar contract as
the main Composer: `Alt+/` slash command, `Alt+R` reference, `Alt+A` attach,
`Mod+Enter` commits the edit, `Esc` opens the discard confirmation, and `Alt+X`
expands. Plain `Enter` remains a newline. Their keycaps appear only while that
Pending region is focused; Mobile renders neither bindings nor hints.

## Visual hierarchy

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

Bare `E` always enters `prompt.composer` and its Vim Normal-mode command sink,
even when Sessions, Conversation, Plan, Queue, or Draft currently owns focus.
Bare `P` enters Plan. When no Plan exists the command remains reserved and
disabled rather than acquiring a transient second meaning.

The Composer is the exception: its caret and outlined editing canvas already
communicate focus, so `prompt.composer` must not receive the generic region
background, accent rail, or focus ring. When Conversation, Sessions, or the
top bar is the highlighted workspace region, the hidden Prompt Vim sink must
not consume typed keys or IME input; those keys belong to the highlighted
chrome and must not pop Prompt into Insert.

Bare `W` cycles visible workspace regions. Prompt Plan, Queue, and Draft are
auxiliary panels rather than Vim windows; enter them through their dedicated
commands, so region cycling never collapses or selects them as an intermediate
stop.

Bare `\` selects the nearest visible vertical boundary and enters layout
Resize mode without moving it: Sessions / Prompt from Sessions, Prompt /
Conversation from either work pane, or Page index / Page in Reading mode.
The selected bar uses the shared accent and keycap language; `H/L` moves it by
16px, `Shift-H/L` moves it by 48px, `Tab` cycles visible bars, and `Esc` or
`Enter` returns to the previously focused region.
Resize mode is exclusive, so unrelated bare keys never leak into lists,
transcript widgets, or destructive actions. Pointer dragging keeps working and
selecting a bar with the pointer enters the same visible state.

Queue and Draft use the same list contract as Sessions: `J/K` selects, `gg` and
`G` jump to the ends, and `1` through `0` jump to one of the first ten visible
slots once that list owns focus. Clicking a visible number does the same jump.
`G` then `1…0` remains available as the sequential form. `L`/`Enter` opens the
selected message editor. `O` pins Order reorder mode
so `J/K` moves the message and `Esc` releases it. Inside the editor, `Mod+Enter`
saves and `Esc` cancels, with both returning focus to the originating list row.
