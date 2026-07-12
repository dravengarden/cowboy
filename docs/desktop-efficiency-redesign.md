# Desktop efficiency redesign

Status: design approved in principle; implementation is staged below.

## Product boundary

Cowboy Desktop and Cowboy Mobile are separate products.

- Desktop optimizes for throughput: keyboard-first operation, dense parallel
  context, visible capability, composable commands, and aggressive use of space.
- Mobile optimizes for touch: single-task focus, large hit targets, progressive
  disclosure, native gestures, and system-keyboard safety.
- Minimalism is not a Desktop goal. Remove visual or interaction layers only when
  doing so reduces time, motion, or cognitive load.
- Shared code stops at protocol, stores, API clients, domain operations,
  attachments, and markdown machinery. Duplicate UI and interaction code is an
  intentional isolation mechanism.

## Current architectural debt

The current split is nominal rather than structural:

- `desktop/DesktopApp.tsx` wraps the shared `App`.
- `desktop/DesktopComposer.tsx` wraps the shared `ComposerWorkspace`.
- Desktop layout, top bar, status bar, session rail, settings, and pane decisions
  remain embedded in `App.tsx`.
- Desktop composer actions remain embedded in the shared `Composer.tsx`.
- The command registry understands single shortcuts but has no focus context,
  multi-key sequences, leader tree, or visible target hints.
- Vim mode is editor-local; the workspace does not have a coherent mode/focus
  model.

The redesign must move ownership, not add more `surface === "desktop"` branches.

## Target workspace

```text
┌ Sessions ┬──────────────────── top command/config bar ────────────────────┐
│ filter   │ session  agent  model  effort  fast  context  follow  stop    │
├──────────┼──────────── Prompt ───────────┬──────── Conversation ──────────┤
│ sessions │ editor                       │ searchable transcript           │
│          │ queue / drafts / plan        │ tool and thought navigation     │
│          │ visible send actions         │                                 │
├──────────┴───────────────────────────────┴─────────────────────────────────┤
│ NORMAL  PROMPT  cowboy  GPT-5.6-Sol  medium  61%  connected   SPC  f  : │
└───────────────────────────────────────────────────────────────────────────┘
```

### Space policy

- Sessions defaults to 272 px and remains independently resizable.
- Conversation is the default primary pane and keeps a readable minimum.
- Prompt defaults to 420 px, is independently resizable, and may be maximized.
- Every pane can be hidden, restored, or maximized without changing product mode.
- Controls expand from the actual container width. A Desktop layout label or
  viewport breakpoint never decides whether useful controls are hidden.
- Dense top-bar controls use available horizontal space before adding menus.

## Interaction model

The user should be fast with basic Vim knowledge and no Cowboy-specific memory.

### Universal motions

Each focused pane implements the same semantic motions where applicable:

| Key | Sessions | Prompt | Conversation |
|---|---|---|---|
| `j` / `k` | next / previous session | Vim motion | scroll or next / previous item |
| `h` / `l` | previous / next pane | Vim motion | previous / next pane |
| `gg` / `G` | first / last session | Vim motion | oldest / newest message |
| `/` | filter sessions | Vim search | search transcript |
| `n` / `N` | next / previous match | Vim search | next / previous match |
| `Enter` | open session | Vim action | expand or activate item |
| `Esc` | clear filter / leave mode | Normal mode | clear selection / Normal mode |

Prompt Insert/Visual/Operator-pending modes always belong to CodeMirror Vim.
Workspace bindings must not steal valid editor Vim sequences.

### Leader board

`Space` in workspace Normal mode opens a LazyVim-style which-key board at the
bottom. It is a command tree and a learning surface, not a list of shortcuts the
user must memorize.

```text
SPC
  s  Session      p  Prompt       c  Conversation
  w  Workspace    a  Actions      g  Go
  f  Find         o  Open         x  Stop / cancel
  ,  Settings     ?  All commands
```

Rules:

- The board appears immediately after a valid prefix.
- Entries show key, icon, action name, availability, and optional explanation.
- Every entry is mouse-clickable.
- The tree is generated from the same command registry as the palette and hints.
- Invalid continuations remain visible with a short reason; they never fail
  silently.
- Dangerous operations open confirmation and never complete from one accidental
  key.

### Vimium hint mode

`f` in workspace Normal mode labels every currently actionable target with a
short, high-contrast key token. This includes top-bar controls, sessions, links,
tool cards, transcript folds, composer actions, tabs, and dialog buttons.

- Prefer stable semantic labels when possible; generate two-character labels on
  collision.
- Typing narrows the visible targets; `Backspace` widens; `Esc` cancels.
- Hints follow elements during scroll and disappear when the element becomes
  unavailable.
- The same target metadata supplies accessible names and command-palette entries.
- Prompt Vim keeps native `f<char>`; workspace hint mode is active outside the
  editor or via Leader `SPC f` while the editor owns Normal mode.

### Persistent key badges

High-frequency fixed controls show a small key badge in workspace Normal mode.
Dynamic page targets receive badges only in hint mode. This gives discoverability
without covering the transcript permanently.

User preference:

- Normal mode only (default)
- Always visible
- Hint mode only

## Top command/config bar

Desktop should expose frequently changed session options inline:

- Agent mode
- Model
- Reasoning effort
- Fast mode
- Context usage and reset detail
- Queue pause/resume
- Follow/auto-scroll
- Stop

Controls are compact select buttons or segmented values, not mobile sheets. They
support mouse selection, arrow-key selection, command execution, leader entries,
and hint targets. Settings retains only low-frequency application preferences and
diagnostics.

## Focus and mode state

Introduce one Desktop workspace controller with:

```ts
type DesktopPane = "sessions" | "prompt" | "conversation";
type WorkspaceMode = "normal" | "leader" | "hint" | "search" | "command";
```

Editor Vim mode is separate (`normal`, `insert`, `visual`, and so on). The
controller derives effective input ownership from focused pane, workspace mode,
editor mode, open modal, and IME composition state.

The status line is always present and shows:

- effective mode and color;
- focused pane;
- active session/provider/model/effort;
- context usage;
- connection and worker state;
- pending key prefix and the next discovery affordance (`SPC`, `f`, or `:`).

## Command platform

Extend the Desktop command model with:

```ts
interface DesktopCommand {
  id: string;
  title: string;
  group: string[];
  icon?: ReactNode;
  leader?: string;
  shortcut?: string;
  contexts?: DesktopPane[];
  danger?: "confirm" | "destructive";
  when?: (context: DesktopContext) => boolean;
  run: (context: DesktopContext) => void;
}
```

One registry powers:

- direct shortcuts;
- leader/which-key;
- command palette and `:` command line;
- persistent badges;
- Vimium hint actions;
- toolbar menus and disabled explanations.

## Target code ownership

```text
desktop/
  DesktopApp.tsx
  DesktopWorkspace.tsx
  DesktopWorkspaceController.tsx
  DesktopTopBar.tsx
  DesktopStatusLine.tsx
  panes/
    DesktopSessionPane.tsx
    DesktopPromptPane.tsx
    DesktopConversationPane.tsx
  commands/
    registry.tsx
    keyResolver.ts
    leaderTree.ts
    CommandPalette.tsx
    CommandLine.tsx
  hints/
    HintProvider.tsx
    HintOverlay.tsx
    HintTarget.tsx
  vim/
    workspaceMotions.ts
    inputOwnership.ts
  preferences/
    desktopPreferences.ts

mobile/
  MobileApp.tsx
  MobileWorkspace.tsx
  MobileComposer.tsx
  MobileNavigation.tsx
  MobileSheets.tsx

shared domain only:
  protocol.ts
  store.ts
  composer/useComposerDraftController.ts
  attachments.ts
  derive.ts
```

Desktop must stop importing `App` and `ComposerWorkspace` when migration is
complete. Shared visual components may be copied into the owning product before
being changed.

## Migration plan

### Phase 1: ownership seam

- Add `DesktopWorkspace` and move the Desktop three-pane shell, top bar, splitters,
  and status line out of `App.tsx` without visual changes.
- Keep Mobile on the current `App` path.
- Move Desktop composer chrome into `DesktopPromptPane`; keep the draft controller
  and editor engine shared.

Acceptance: Desktop no longer renders through shared responsive layout branches;
Mobile screenshots and bundle contents are unchanged.

### Phase 2: workspace focus and status

- Add pane focus state and visible active-pane treatment.
- Make the status line always present.
- Implement basic `j/k`, `gg/G`, `/`, `Enter`, and `Esc` semantics outside the
  editor.
- Preserve CodeMirror Vim ownership inside the editor.

Acceptance: the primary Desktop workflow is possible without a pointer and basic
Vim keys behave consistently.

### Phase 3: command unification and leader

- Replace the fixed shortcut-only host with contextual command registration.
- Build the leader tree and which-key board from that registry.
- Move every existing Desktop toolbar/menu action into the registry.

Acceptance: every fixed Desktop action is available from both UI and leader;
disabled actions explain why.

### Phase 4: Vimium hints

- Add target registration and overlay positioning.
- Cover top bar, session rows, transcript links/cards, composer actions, settings,
  and dialogs.
- Add focus restoration and scroll-safe target updates.

Acceptance: any visible clickable control can be activated without a pointer and
without memorizing a Cowboy command.

### Phase 5: dense Desktop top bar and panes

- Inline all high-frequency agent/session configuration.
- Add pane maximize/hide/reset and persisted widths.
- Add transcript search/navigation and session filtering.
- Remove Desktop use of mobile sheets and responsive action folding.

Acceptance: model/effort/mode changes, session switching, transcript navigation,
prompt sending, queue operations, and stop are keyboard-complete.

### Phase 6: hard isolation

- Delete obsolete Desktop branches from shared `App.tsx` and `Composer.tsx`.
- Verify Mobile chunks contain no Desktop leader, hint, or command code.
- Keep duplicated UI code where ownership differs.

Acceptance: Desktop and Mobile can change layout independently with no shared
responsive conditionals.

## Verification gates

Each phase must pass:

- TypeScript, Oxlint, Deno tests, and production build.
- Desktop Chrome checks at 1100, 1440, 1728, and ultrawide sizes.
- Keyboard-only scripted workflows for session selection, pane focus, command
  execution, prompt editing/sending, transcript search, and cancel/escape.
- No horizontal overflow at persisted minimum/maximum pane widths.
- No shortcut handling during IME composition.
- Mobile 390 px and touch-tablet regression checks; no Desktop chunks loaded.
- Web-only deployment with unchanged core, agentd, and ACP worker PIDs.
