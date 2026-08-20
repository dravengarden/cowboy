# Desktop efficiency redesign

Status: architectural direction; the live interaction contract in
[`web/src/desktop/FOCUS.md`](../web/src/desktop/FOCUS.md) is normative.

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
- Desktop visual primitives should use MUI semantics and theme tokens wherever
  MUI already has the right interaction model. Custom UI is reserved for
  editor/workspace primitives that MUI does not provide: splitters, Vim status,
  shortcut bars, and sequential-chord state.

## Current architectural debt

The current split is nominal rather than structural:

- `desktop/DesktopApp.tsx` wraps the shared `App`.
- `desktop/DesktopComposer.tsx` wraps the shared `ComposerWorkspace`.
- Desktop layout, top bar, status bar, session rail, settings, and pane decisions
  remain embedded in `App.tsx`.
- Desktop composer actions remain embedded in the shared `Composer.tsx`.
- The command registry understands single shortcuts but still needs complete
  focus-context coverage and shared sequential commands.
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
│ NORMAL  PROMPT  cowboy  GPT-5.6-Sol  medium  61%  connected   g·  ⌘K  ⌘/ │
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

### Direct discovery and sequential commands

Cowboy does not add a Space leader layer. Basic Vim motion, a small stable
workspace prefix (`Cmd+K` on macOS, `Alt+K` on Windows/Linux), contextual key
slots, `Mod+Shift+P` Command Palette, and `Mod+/` shortcut guide remain visible
and usable from editor Insert/Normal/Visual modes. Global bare product letters
are forbidden; bare letters belong only to an explicitly focused non-editor
context.

Every live key slot uses one state machine:

- `inactive`: the displayed action cannot execute in the current scope/state;
- `available`: the chord executes now;
- `active`: a transient command owner or prefix is engaged.

Selection and focus use their own control styling; they are never represented
as an active keycap. `Enter` activates a focused item, `Mod+Enter` confirms or
commits a mutation, and `Esc` unwinds one innermost transient layer. Navigation-
rich surfaces use one shared bottom shortcut bar; simple confirmations keep
their two chords beside the actions.

Sequential commands such as `G` then `1…0` keep the prefix available and the
continuations inactive at rest. Pressing the prefix makes it active and exposes
only valid continuations as available. A valid continuation executes; `Esc`, an
invalid unmodified continuation, timeout, or focus change cancels. Modified
global commands cancel the prefix but keep executing. The executable details
and accessibility contract live in `web/src/desktop/FOCUS.md`.

### No page-wide target overlays

Cowboy Desktop never renders a Vimium-style target-hint layer over the working
surface and never reserves bare `f` for one. A dense page can expose hundreds
of targets; covering the content with generated key tokens raises visual and
cognitive load and conflicts with Prompt Vim's native `f<char>` motion.

Keyboard reachability comes from native focus order, direct Vim motions,
visible contextual shortcuts, and `Mod+Shift+P` Command Palette. New controls must
join those mechanisms instead of adding a page-wide overlay.

### Persistent key badges

High-frequency fixed controls show a small key badge in workspace Normal mode.
Dynamic page content never receives floating generated badges. This preserves
discoverability without covering the transcript.

Product-mode transitions are visible fixed controls rather than palette-only
knowledge. In the Conversation header, Reading is a separate embedded action—not
a History/Explore projection—and its contextual `Z` slot truthfully reflects
whether Conversation currently owns keyboard focus. Entering Reading preserves
the projection; `Esc` returns to Agent.

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
support mouse selection, arrow-key selection, command execution, and contextual
shortcut slots. Settings retains only low-frequency application preferences and
diagnostics.

## Focus and mode state

Introduce one Desktop workspace controller with:

```ts
type DesktopPane = "sessions" | "prompt" | "conversation";
type WorkspaceMode = "normal" | "sequence" | "search" | "command";
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
- pending key prefix and the next discovery affordance (`g·`, `⌘K`, or `⌘/`).

## Command platform

Extend the Desktop command model with:

```ts
interface DesktopCommand {
  id: string;
  title: string;
  group: string[];
  icon?: ReactNode;
  shortcut?: string;
  sequence?: readonly string[];
  contexts?: DesktopPane[];
  danger?: "confirm" | "destructive";
  when?: (context: DesktopContext) => boolean;
  run: (context: DesktopContext) => void;
}
```

One registry powers:

- direct and sequential shortcuts;
- command palette;
- persistent badges;
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
    shortcutAvailability.ts
    CommandPalette.tsx
    CommandLine.tsx
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

### Phase 3: command unification and shortcut grammar

- Replace the fixed shortcut-only host with contextual command registration.
- Build shared inactive/available/active key slots and shortcut bars from that
  registry.
- Move every existing Desktop toolbar/menu action into the registry.

Acceptance: every fixed Desktop action is available from both UI and Command
Palette/status discovery;
disabled actions explain why.

### Phase 4: native focus and direct action coverage

- Give every actionable control a stable accessible name and native focus path.
- Cover high-frequency actions with direct contextual shortcuts.
- Register remaining actions in Command Palette with clear scope and disabled
  explanations.

Acceptance: every visible clickable control can be reached through native focus,
a direct contextual command, or Command Palette, with no page-wide hint overlay.

### Phase 5: dense Desktop top bar and panes

- Inline all high-frequency agent/session configuration.
- Add pane maximize/hide/reset and persisted widths.
- Add transcript search/navigation and session filtering.
- Remove Desktop use of mobile sheets and responsive action folding.

Acceptance: model/effort/mode changes, session switching, transcript navigation,
prompt sending, queue operations, and stop are keyboard-complete.

### Phase 6: hard isolation

- Delete obsolete Desktop branches from shared `App.tsx` and `Composer.tsx`.
- Verify Mobile chunks contain no Desktop shortcut-bar or command code.
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
- Web-only deployment with unchanged core, Machine broker, and ACP worker PIDs.
