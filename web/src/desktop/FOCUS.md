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

## Navigation

- `Ctrl-w h/l`: adjacent pane.
- `Ctrl-w j/k`: adjacent region in the current pane.
- `Ctrl-w w`: cycle every visible region.
- `j/k`, `gg`, `G`: item navigation outside text-editing controls. Conversation
  is a reader rather than an item list, so the same keys scroll by line or jump
  to the oldest/latest output there.
- In Sessions, the filled row remains the currently open session; a distinct
  accent cursor shows the row selected by `j/k` for the next `l`/Enter action.
- `Enter`: default item action. In Sessions, `l` and `Enter` open the selected
  session and move focus to its Prompt editor; entering the Sessions region
  always starts on the currently open session.
- Sessions use `p` to enter or leave Pin reorder mode. While pinned, `j/k`
  moves the selected session instead of moving selection, and `Esc` releases
  the mode. `h` opens the selected session's Settings directly; the trailing
  three-dot menu retains secondary actions such as Rename and Delete.
- `i`: edit the item when it exposes an edit action.
- `Esc`: close the current transient layer or leave editor Insert mode.
- `Alt-b/p/c/t`: jump directly to Sessions, Prompt, Conversation or Top Bar.
- `p`, `Mod-y`, and `Mod-d` jump to Plan, Queue, and Drafts after Prompt owns
  focus. `Mod-i` returns to the Composer in Normal mode from anywhere. The
  composer keeps native Vim letter commands. One scoped exception keeps the
  visible Plan hint honest: `p` may leave a completely empty Normal-mode
  composer for Plan; once the editor has text or attachments, it is Vim paste.
- `Mod-y` is also the Queue disclosure action: from another Prompt region it
  expands and focuses Queue; from Queue it collapses the panel and returns to
  the Composer in Normal mode.
- `Mod-1…0`: jump to one of the first ten visible items in the focused region.
  In Sessions this switches the active session immediately while retaining
  focus in the rail, so the next `j/k` continues from the selected row.
- `Mod-j/k`: reorder the focused Queue or Draft item. Sessions use the more
  deliberate Pin mode above so ordinary navigation and reordering cannot be
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
  presents newest questions first, and uses Vim list navigation: `j/k` moves
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
- `Shift-f` toggles automatic following; bare `f` opens Vimium-style target
  hints for every visible clickable control, including dialogs.
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
entry or `Mod+/`. `Mod+k` opens the all-command palette.

Bare `f` is the pointerless escape hatch: it overlays short home-row labels on
every visible actionable control. Type a label to focus and activate that
target, or press Escape to cancel. Labels are recomputed while scrolling and
when dialogs mutate, so the same mechanism covers the workspace, Tool History
inspector, menus, and modal actions without every control needing a bespoke
shortcut.

Shortcut hints follow one shared keycap grammar and three visibility levels:

1. global shortcuts are always visible but quiet (`Mod+E/I/L/T`, `Mod+N`,
   `Mod+,`) because they work without first focusing a region;
2. contextual shortcuts float over their action only while the owning region is
   focused (Prompt subregions and list item actions), so they add no layout
   width and disappear when attention moves elsewhere;
3. modal actions show their real confirmation/dismissal chord next to the label.

Embedded contextual shortcuts are the persistent exception to visibility
gating. Controls such as Top Bar Run Configuration, Usage, Compact, and Stop
keep their one-key badge visible for discovery, but the shared keycap primitive
must render it as `data-shortcut-state="inactive"` whenever its owning region
is not focused. Once the region owns focus it becomes
`data-shortcut-state="available"` and gains the normal accent treatment. Do not
approximate these states with component-local opacity or colors; all persistent
contextual badges must use `ShortcutKeycap` availability so enabled and inactive
semantics remain identical across Desktop.

Never invent a hint for an action that is not wired. Contextual hints anchor to
the bottom-right of their target. Prefer one primary modifier and one positional
or mnemonic key while retaining native semantic chords such as `Mod+S`. Put secondary Cowboy-specific actions in
the searchable command palette. Shared modal shells may use the same visual
primitive, but must hide it on the touch product.

### macOS collision policy

Every Desktop shortcut must pass the checked-in macOS collision audit before
it can enter the command registry. The policy follows Apple's system shortcut
contract and the conventions users depend on in editors and browsers:

- never capture destructive or system-owned chords such as `Mod+Q/W/H/M`,
  `Mod+Tab`, `Mod+Space`, `Ctrl+Mod+Q/F`, `Alt+Mod+Esc`, or
  `Shift+Mod+3/4/5`;
- common application chords may be used only with matching native semantics:
  `Mod+N` creates a session, `Mod+,` opens Settings, `Mod+S` saves a draft,
  and `Mod+1…0` switches session tabs;
- global Cowboy workspace navigation uses mnemonic `Mod+E/I/L/T` for Sessions,
  Editor, Log, and Top Bar. Focused Top Bar uses bare `R/U/C/S` for Run
  Configuration, Usage, Compact, and Stop; `Mod+1…0` remains global session slots;
- macOS input-source and dead-key chords (`Ctrl+Space`, `Ctrl+Alt+Space`,
  `Alt+E/I/N/U`) are reserved so Cowboy never breaks accent or IME entry;
- do not assign bare `Q` to Cowboy navigation. A key pressed while Command is
  being released can otherwise become `Mod+Q` and quit the native shell;
- Vim-native keys remain unmodified and are active only in Normal mode or the
  focused non-editor region. IME composition and text editing always win.

`macShortcutPolicy.ts` is the executable source of truth. A conflicting
registered command throws during development and fails the frontend test gate;
update the policy and this document together when intentionally adding an
exception. The reference inventory is Apple's official Mac keyboard-shortcut
guide: <https://support.apple.com/en-us/102650>.

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

The Composer is the exception: its caret and outlined editing canvas already
communicate focus, so `prompt.composer` must not receive the generic region
background, accent rail, or focus ring.

`Ctrl-K` and `Ctrl-J` move vertically between the active pane and
`topbar.controls`. Prompt Plan, Queue, and Draft are auxiliary panels rather
than Vim windows; enter them through their dedicated commands, so vertical
window movement never collapses or selects them as an intermediate stop.

Queue and Draft use the same list contract as Sessions: `J/K` selects, `gg` and
`G` jump to the ends, `g1` through `g0` jump to a visible slot, and `L`/`Enter`
opens the selected message editor. `P` pins reorder mode so `J/K` moves the
message and `Esc` releases it. Inside the editor, `Mod+Enter` saves and `Esc`
cancels, with both returning focus to the originating list row.
