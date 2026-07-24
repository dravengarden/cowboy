# Explore transcript projection

Status: product and interaction design  
Scope: Cowboy web, iPhone, iPad, and Desktop  
Non-goal: changing ACP history, the native Codex thread, or the existing History UI

## 1. Product model

Cowboy keeps one canonical session event timeline and offers two projections:

- **History** is the current transcript. It preserves the existing chronology,
  composer, queue, draft, tools, scroll behavior, and controls.
- **Explore** groups the same events into question pages for focused reading.

Changing projection is local presentation state. It must not send a prompt,
rewrite an event, move an event, change the main queue, or create a model-visible
mode marker.

```text
Session
├── canonical event timeline
├── History projection
└── Explore projection
    └── Question pages
```

Forks, isolated inquiries, and a future tree projection are intentionally
deferred. This release does not create child sessions or introduce new ACP
semantics, which keeps the canonical session fully compatible with History.

## 2. Invariants

1. History must render exactly as it does before this feature. No Page Dock,
   page count, empty page, or page-specific composer is mounted in History.
2. History and Explore retain independent scroll state per session.
3. A projection switch never changes the selected session or main-thread state.
4. New devices derive the same automatic page boundaries from the same events.
5. User corrections to page grouping are synchronized as projection metadata.
6. Explore does not create forks, child sessions, or hidden model turns.

## 3. Question-page derivation

A page is a contiguous group of main-thread question turns:

```ts
type QuestionPage = {
  id: string;
  rootUserEventId: string;
  turns: QuestionTurn[];
  status: "queued" | "answering" | "completed" | "failed";
};

type QuestionTurn = {
  userEventId: string;
  eventStart: string;
  eventEnd: string | null;
  relation: "root" | "follow_up";
};
```

The original event order is never changed. A page stores stable event
boundaries, not copied transcript text.

### 3.1 Deterministic default grouping

Create a new page for the first user prompt after:

- session creation;
- a clear/reset boundary;
- an explicit `New page` submission;
- a user correction that splits the projection.

Append a prompt to the current page when:

- it was submitted as `Follow up`;
- it was started from a selection inside that page;
- it follows the page's answer without an explicit new-page intent.

Do not call a model merely to decide page boundaries. The deterministic
projection is instant, offline-capable, and identical across devices.

### 3.2 Manual correction

Every root or follow-up question supports:

- `Start new page here`;
- `Merge with previous page`;
- `Move to page…`.

These commands modify projection metadata only. History remains unchanged.

## 4. Mobile Explore

Explore is a reading surface. It does not keep the normal History composer
mounted underneath every page.

### 4.1 Page canvas

- One question page occupies the reading viewport.
- A page may contain multiple consecutive question/answer turns.
- Long follow-ups may be collapsed independently, but the root turn stays open.
- Horizontal swiping changes pages only when the gesture begins outside
  selectable content and locks clearly to the horizontal axis.
- Vertical reading and native selection always win over page swiping.
- Page changes restore the saved scroll position for that page.

### 4.2 Mobile Page Dock

The bottom Page Dock sits immediately above the global Session Bar, where the
thumb already expects the primary action surface. On the latest page the normal
composer sits above the Dock. On an older page the composer is not mounted
until the user explicitly starts a follow-up or new question, maximizing the
reading viewport. The Dock is not placed on the left or right edge, does not
auto-hide, and does not float in the reading column.

```text
╭─────────────────────────────────────────────────╮
│ [ 5 / 18 · Prompt caching ▴ ]  [ ‹ │ › ]  [ ＋ ] │
╰─────────────────────────────────────────────────╯
```

The layout is deliberately asymmetric for one-handed efficiency:

- the large page summary is a forgiving target for arbitrary navigation;
- Previous and Next form one segmented control near the dominant thumb zone;
- New question is the trailing primary action;
- Next is closer to New question than Previous because forward movement is the
  common reading action.

At very narrow widths, the title moves above the controls while the controls
keep their positions:

```text
  Prompt caching · 3 questions
╭───────────────────────────────────╮
│ [ 5 / 18 ▴ ]  [ ‹ │ › ]    [ ＋ ] │
╰───────────────────────────────────╯
```

Rules:

- Tapping the page summary opens Page Navigator.
- Previous, Next, and New question use at least 44-point touch targets.
- The controls never move when their enabled state changes.
- At the first page, Previous remains in its slot and is visibly disabled.
- The Dock contains page summary/Navigator, Previous, Next, Follow up, and New
  question. Stable slots prevent controls moving as navigation state changes.
- A completed later page adds a quiet unread dot inside Next; it never moves the
  viewport.
- The Dock reserves one stable bottom inset in Explore. It does not overlap
  prose, animate transcript padding, collapse on scroll, or produce a layout
  jump.
- A horizontal swipe remains an optional accelerator, not the only way to move.

### 4.3 Mobile Page Navigator

Page Navigator is a bottom-up sheet based on the Session picker interaction,
not a menu.

Compact detent:

- sheet title and page count;
- search field;
- current page centered in a short result window.

Expanded detent:

- all pages;
- question title;
- two-line answer preview;
- follow-up count;
- queued, answering, completed, failed, and unread status;
- a final `New question` row.

The sheet initially scrolls the current page to the visual center. Selecting a
row closes the sheet and restores that page's last reading position.

### 4.4 New-question page

The final virtual page is `New question`. It is not an ACP event until sent.

- It uses Cowboy's existing mobile composer and attachment machinery.
- Tapping New question opens the composer immediately above the Page Dock.
- Enter inserts a newline.
- The existing send action submits.
- Submission seals the virtual page and immediately creates the next empty page.
- The user may choose `Stay here` or `Read when ready`; default is to stay on
  the page they were reading before opening New question.
- A follow-up launched inside an existing page defaults to `Follow up`.
- Closing an untouched composer leaves only the Page Dock. Closing a non-empty
  composer uses the existing draft behavior.

## 5. iPad Explore

iPad is touch-first but has enough width for parallel context.

Portrait:

- same Page Dock and bottom Page Navigator as iPhone;
- wider readable measure, not full-width prose;
- Navigator may expand to a near-full-height sheet.

Landscape:

- page canvas remains the primary reading column;
- Page Navigator opens as a 360–420 px right-side sheet;
- opening it does not resize the page canvas or lose selection;

Hardware keyboard:

- `J/K`: previous/next page when focus is on the page navigator;
- `Enter/L`: open selected page;
- `/`: focus page search;
- `Esc/H`: close the active sheet;
- shortcuts use physical `KeyboardEvent.code` when IME is active.

## 6. Desktop Explore

Desktop uses the available horizontal space and does not imitate the mobile
sheet.

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Conversation · Explore        Page 5 / 18        Search  New page   │
├───────────────┬───────────────────────────────────────┬──────────────┤
│ Page index    │ Question page                         │ Page outline │
│               │                                       │              │
│  3  ACP       │ Q  Prompt cache prefix               │ Q1 root      │
│  4  Fork      │ A  …                                  │ Q2 follow-up │
│▌ 5  Cache     │                                       │ Q3 follow-up │
│  6  Desktop   │ Q  But what about ACP?               │              │
│  7  Aside     │ A  …                                  │              │
│               │                                       │              │
└───────────────┴───────────────────────────────────────┴──────────────┘
```

### 6.1 Adaptive panes

- At wide widths, show page index, reading canvas, and page outline together.
- At medium widths, keep page index and reading canvas; outline becomes a
  toggleable inspector.
- At the Desktop minimum width, page index becomes a command-palette modal.
- The main reading measure remains bounded; extra width belongs to navigation,
  outline and source context—not longer prose lines.

### 6.2 Keyboard model

When Explore owns Conversation focus:

- `J/K`: next/previous page;
- `G/GG`: last/first page;
- `Enter/L`: open the focused page or follow-up;
- `H`: return from outline/detail to its parent region;
- `/`: search pages;
- `N`: new question;
- `F`: follow current answering page;
- `Tab` and `Shift+Tab`: move between index, page, and outline;
- `Esc`: close the topmost overlay, otherwise return to History.

The existing global focus shortcuts remain available. Overlay command scopes
must override workspace and global single-key commands.

Every active shortcut is shown in the owning widget without moving surrounding
widgets. Disabled hints remain in the same internal slot at reduced contrast.

## 7. Projection switch

### 7.1 Mobile

Place the projection control in the existing session-options detent sheet. It
is an explicit `History | Explore` segmented choice, so the global Session Bar
does not gain another always-visible icon and History keeps its original chrome.

### 7.2 Desktop

The Conversation pane header owns a compact two-option control:

```text
CONVERSATION     [History  Explore]
```

The switch does not move the pane title, follow state, or shortcut slots.

## 8. Persistence and synchronization

Local, per-device:

- selected projection;
- current page;
- page scroll positions;
- collapsed follow-up sections;
- Navigator detent.

Server-synchronized:

- explicit page split/merge/move metadata;
- unread completion state.

Ephemeral:

- open menus and sheets;
- hover/focus styling;
- swipe progress.

Projection metadata must tolerate missing or late history. A page renders a
loading skeleton for unresolved event ranges and fills in place without
changing the user's current page.

## 9. Prompt-prefix discipline

Explore is presentation-only, so changing projection never changes the model
prefix.

- Main questions continue on the native parent session.
- Do not prepend page numbers, projection names, UI state, or repeated summaries
  to main prompts.
- Future tree mode must be introduced as a separate projection and explicit
  session model; Explore must not anticipate it with hidden fork metadata.

## 10. Accessibility

- Page Dock is a `navigation` landmark named `Question pages`.
- Page Navigator is a labelled dialog/sheet with a real listbox.
- Status is never color-only; every state has visible text and an accessible
  label.
- Touch targets are at least 44×44 CSS px.
- Dynamic page completion is announced politely and never moves focus.
- Projection switches announce the destination and preserve logical focus.
- Reduced Motion and increased font scale must not hide navigation or truncate
  the active question beyond recovery.

## 11. Delivery slices

### Slice A — projection foundation

- deterministic page derivation;
- per-session projection state;
- History remains bit-for-bit behaviorally unchanged;
- manual page split/merge metadata schema.

### Slice B — Mobile and iPad

- Page canvas;
- bottom Page Dock;
- bottom/right Page Navigator;
- New-question page;
- scroll restoration and orientation acceptance.

### Slice C — Desktop

- adaptive index/canvas/outline;
- scoped Vim navigation;
- searchable Page Palette;
- visible stable shortcut slots.

## 12. Acceptance scenarios

1. Switch History → Explore → History while an answer streams. History events,
   scroll position, composer text, queue, and tool states are unchanged.
2. Read Page 3 while Page 8 completes. The viewport stays on Page 3 and exposes
   a quiet ready indicator.
3. Open Navigator with 100 pages. It centers the current page and jumps to any
   result without rendering the entire History projection.
4. Rotate iPhone/iPad with Navigator, keyboard, and New question open. No blank
   viewport, double inset, or lost draft occurs.
5. Open the same session on Desktop and iPhone. Explicit page grouping changes
   synchronize; each device keeps its own current page and scroll positions.
6. Use Desktop without a mouse: switch projection, search pages, open Page 12,
   navigate its follow-ups, and return to History.
