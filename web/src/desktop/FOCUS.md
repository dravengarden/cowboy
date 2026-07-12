# Desktop focus and keyboard contract

Desktop is a Vim-first productivity surface. Mobile does not load this focus
controller, command registry, leader board, or shortcut guide.

## Hierarchy

Keyboard focus has three levels:

1. pane: Sessions, Prompt, Conversation;
2. region: list/editor/transcript or Prompt's Queue, Drafts and Composer;
3. item: a session, queued prompt, draft, or transcript entry.

DOM integration uses `data-desktop-pane`, `data-desktop-region`, and
`data-desktop-item`. A region may mark its preferred real focus target with
`data-desktop-focus-default`. Mouse focus and keyboard navigation update the
same controller state.

## Navigation

- `Ctrl-w h/l`: adjacent pane.
- `Ctrl-w j/k`: adjacent region in the current pane.
- `Ctrl-w w`: cycle every visible region.
- `j/k`, `gg`, `G`: item navigation outside text-editing controls.
- `Enter`: default item action.
- `i`: edit the item when it exposes an edit action.
- `Esc`: close the current transient layer or leave editor Insert mode.

Text inputs and CodeMirror retain their own Vim/IME semantics. Workspace list
navigation must never intercept unmodified keys while a text-editing target
owns focus.

## Commands and help

Commands may declare pane `contexts` and exact `regions`. The leader board
filters contextual commands but always keeps global commands. The Command
Palette searches every registered command.

The Desktop-only shortcut dialog opens from the status-line `? Shortcuts`
entry, `Mod+/`, or `SPC h k`. `SPC ?` remains the all-command entry.

## Visual hierarchy

Only the focused region gets the subtle accent rail/background. The focused
item uses the MUI selected/focus-visible treatment. Avoid simultaneous heavy
outlines at pane, region and item levels.
