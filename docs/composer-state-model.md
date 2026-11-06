# Composer workspace state model

Status: proposal (2026-11-06). Follows Phase 3.4 of
[`structural-refactor-plan-2026-11.md`](structural-refactor-plan-2026-11.md),
which moved everything out of `ComposerWorkspace` that could leave by a pure
move. What remains is about 2,300 lines whose local state is read by handlers,
effects and several JSX regions at once. This document records that state, the
constraints any new model must keep, and the order in which it can change
safely. It changes no behaviour by itself.

## Inventory

Local state in `ComposerWorkspace` (`web/src/Composer.tsx`), grouped by what it
represents. Reads are occurrences in the file at the time of writing.

| Group | State | Reads | Opened by | Closed by |
|---|---|---|---|---|
| Surfaces | `composeFs` (full-screen editor) | 8 | expand tap (`flushSync`) | collapse, delivery success, save draft, context reset |
| | `deliveryAnchor` (delivery picker) | 5 | send with a queue/busy turn | any choice, stale draft, session change |
| | `cancelOpen` (stop confirmation) | 4 | Esc-to-stop, picker "Stop" | confirm, close |
| | `clearComposerAnchor` (clear-draft popper) | 5 | clear button | click-away, confirm, context reset |
| | `cmdConfirm` (compact/clear context dialog) | 6 | toolbar actions | confirm, close |
| | `scheduleTarget` (schedule dialog) | 9 | toolbar, full-screen, pending rows | close, save |
| | `moveSrcId` (move-draft source) | 3 | pending/draft rows | move surfaces |
| | `mobileToolbarSettingsOpen` | 2 | mobile toolbar | close |
| Mobile keyboard | `mobileKeyboardDismissed`, `mobileInputResetBlocked` | 3 each | keyboard dismissal, context reset | refocus, timeline change |
| Mobile editing | `mobileQueuedEditing`, `mobileDraftEditing` | 2 each | pending panel callbacks | same |
| Gesture | `holding`, `lpFired`, `mobileActionEdges` | 3 / – / 7 | long-press, layout | release, layout |
| Plan banner | `dismissedPlanKey` | 2 | dismiss | new plan key |

The delivery picker's choice guard (`deliverySource.ts`) already lives outside
the component with its own tests.

## Constraints a new model must keep

1. **Surfaces overlap.** The delivery picker opens from the full-screen
   editor's send button, and its "Stop" choice opens the stop confirmation in
   the same handler. A single "which surface is open" union is wrong; at least
   the full-screen editor is orthogonal to the modal surfaces.
2. **Full-screen transitions are synchronous.** Expand and collapse snapshot the
   editor selection, call `beginMobileEditorFocusTransfer()`, commit with
   `flushSync`, and restore the selection inside the same user gesture. UIKit
   otherwise ends its keyboard transaction and loses the caret. A reducer may
   own the state, but the dispatch must stay inside `flushSync` at these call
   sites, never in an effect.
3. **Closing the picker restores focus synchronously** (`flushSync` before
   `focusSelection`), pinned by `mobileEditorFocusContract.test.ts`.
4. **Context reset is a hard end of input on touch devices**: it closes the
   full-screen editor, the clear popper and the image selection, blocks input
   reset and releases focus twice (now and next frame).
5. **Session change closes only the picker** and forgets its source; the other
   surfaces are keyed by the parent remounting the workspace.

## Proposal

Move the eight surface states into one `useComposerSurfaces` hook backed by a
pure reducer:

```ts
interface ComposerSurfaces {
  fullscreen: boolean;
  modal:
    | { kind: "none" }
    | { kind: "delivery"; anchor: HTMLElement }
    | { kind: "stop" }
    | { kind: "clear"; anchor: HTMLElement }
    | { kind: "context"; action: SessionAction }
    | { kind: "schedule"; target: ScheduleTarget }
    | { kind: "move"; sourceId: string }
    | { kind: "toolbarSettings" };
}
```

`fullscreen` stays independent (constraint 1). Every modal transition then
closes the previous modal by construction, which today depends on each handler
remembering to (`onStop` clears the picker before opening the stop dialog).
Events are named after user intent (`expand`, `collapse`, `delivered`,
`openDelivery`, `chooseStop`, `contextReset`, `sessionChanged`), so constraint
4 and 5 become reducer cases with table tests instead of handler bodies.

Before adopting the single `modal` slot, confirm that no two of these modals
are ever open together on purpose; the inventory found none, but the move-draft
undo snackbar is owned by `DraftMoveSurfaces` and must stay outside the slot.

The mobile keyboard and gesture groups stay as they are: they track platform
state rather than user-visible surfaces, and every change there needs physical
iOS acceptance (web/AGENTS.md pitfall #69).

## Order of work

1. Reducer and table tests only (`composerSurfaces.ts`), unused.
2. Switch the surfaces one at a time, starting with those that do not touch
   focus (`scheduleTarget`, `cmdConfirm`, `moveSrcId`, toolbar settings), then
   `cancelOpen` and the picker, and `composeFs` last.
3. After each switch: web unit tests, the `desktop-composer`, `sheet-keyboard`
   and `managed-calls` CDP suites, and `just desktop-perf-profile` before and
   after (web/AGENTS.md).
4. `composeFs` and the picker additionally need physical iOS composer
   acceptance before release; until it has been run, those two stay on
   `useState`.

## Progress

- 2026-11-06: steps 1 and the focus-free part of step 2 landed.
  `web/src/composerSurfaces.ts` owns one modal slot for the schedule dialog,
  the context (compact/clear) confirmation, the move-draft picker and the
  mobile toolbar settings; a close names its surface, so a replaced dialog's
  late close cannot dismiss its successor. The slot is still named
  `ComposerModal`. The stop confirmation and the non-modal clear-draft popper
  joined the same day; the picker and `composeFs` wait for physical iOS
  acceptance.
- `just cdp-browser-conformance <endpoint> composer-surfaces` drives these
  surfaces in the real App (Desktop and touch layouts, in-page fake socket and
  fetch): schedule open/dismiss, picker Stop replacing the picker with the stop
  confirmation and sending `cancel`, the clear-draft popper (Escape keeps the
  draft, confirm clears it), the clear-conversation confirmation, the mobile
  toolbar options and the move-draft picker (the fake socket acknowledges
  `add_draft` with a `sync_patch`; dismissing moves nothing, a destination
  sends `move_draft`). Removing the picker's Stop transition, the popper's
  Escape close, the move entry or the toolbar options close fails it (checked).
  Synthetic Escape is not trusted input for the Desktop key dispatcher or the
  move sheet, so that suite dismisses those two by backdrop or Close.
- `bun tools/cdp-keyboard-acceptance.ts <endpoint> <out> surfaces` (also part of
  `just desktop-keyboard-acceptance`) mounts the same Desktop App and sends a
  trusted Escape to the schedule sheet, the delivery picker and the move-draft
  picker. Closing the picker must return focus to the Prompt editor and keep
  the draft; removing `closeDeliveryPicker`'s focus restoration fails it
  (checked).
  The older desktop-composer suite renders the toolbar alone and never mounted
  `ComposerWorkspace`.

## Non-goals

Changing what any surface does, merging the desktop and mobile composer paths,
or replacing the source-text contract tests with behaviour tests in the same
change.
