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
- `Enter`: default item action. In Sessions, `l` and `Enter` open the selected
  session and move focus to its Prompt editor; entering the Sessions region
  always starts on the currently open session.
- `i`: edit the item when it exposes an edit action.
- `Esc`: close the current transient layer or leave editor Insert mode.
- `Mod-b/e/t/u`: jump directly to Sessions, Prompt, Conversation or Top Bar.
- `p/q/d/e`: jump to Plan, Queue, Drafts or Editor after Prompt owns focus.
  The composer keeps native Vim `p/q/d/e`; these region jumps are available
  only while a non-editor Prompt region owns focus. One scoped exception keeps
  the visible Plan hint honest: `p` may leave a completely empty Normal-mode
  composer for Plan; once the editor has text or attachments, it is Vim paste.
- `q` is also the Queue disclosure action: from another non-editor Prompt
  region it expands and focuses Queue; from Queue it collapses the panel and
  returns to the Composer in Normal mode.
- `Mod-1…0`: jump to one of the first ten visible items in the focused region.
  In Sessions this switches the active session immediately while retaining
  focus in the rail, so the next `j/k` continues from the selected row.
- `Mod-j/k`: reorder the focused item when its region is reorderable.
- Inside Prompt, `E` returns from Plan, Queue, or Drafts to the main editor and
  lands on the Desktop Vim command sink in Normal mode.

Text inputs and CodeMirror retain their own Vim/IME semantics. Workspace list
navigation must never intercept unmodified keys while a text-editing target
owns focus.
Workspace Vim letters are resolved from physical `Key*` codes so a non-Latin
macOS input source cannot turn `j/k`, `gg/G`, `l`, or `i` into marked text.

## Conversation reader

The Conversation header owns the visible `Following`/`Follow` control. It is
session state, not a global top-bar action. While Conversation is focused:

- `j/k` scroll down/up by one reading line;
- `Ctrl-d/u` scroll down/up by half a page;
- `Ctrl-f/b` scroll down/up by one page;
- `gg/G` jump to the oldest/latest output;
- `f` toggles automatic following.
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

Shortcut hints follow one shared keycap grammar and three visibility levels:

1. global shortcuts are always visible but quiet (`Mod+B/E/T/U`, `Mod+N`,
   `Mod+,`) because they work without first focusing a region;
2. contextual shortcuts float over their action only while the owning region is
   focused (Prompt subregions and list item actions), so they add no layout
   width and disappear when attention moves elsewhere;
3. modal actions show their real confirmation/dismissal chord next to the label.

Never invent a hint for an action that is not wired. Contextual hints anchor to
the bottom-right of their target. Prefer one modifier plus one physical key
(`Mod+S`, `Alt+A`) so commands stay fast and work across input sources; avoid
browser-owned chords such as `Mod+D`. Put secondary Cowboy-specific actions in
the searchable command palette. Shared modal shells may use the same visual
primitive, but must hide it on the touch product.

The inline queued/draft editor follows the same discoverable toolbar contract
as the main Composer: `Alt+/` slash command, `Alt+R` reference, `Alt+A` attach,
`Mod+Enter` finish editing, and `Alt+E` expand. Their keycaps appear only while
that Pending region is focused; Mobile renders neither bindings nor hints.

## Visual hierarchy

Only the focused region gets the subtle accent rail/background. The focused
item uses the MUI selected/focus-visible treatment. Avoid simultaneous heavy
outlines at pane, region and item levels.

The Composer is the exception: its caret and outlined editing canvas already
communicate focus, so `prompt.composer` must not receive the generic region
background, accent rail, or focus ring.
