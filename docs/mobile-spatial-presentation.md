# Mobile spatial presentation

Status: core product contract  
Scope: Cowboy Mobile PWA — Sessions/Review drawers, product pager, Transcript
paint budget, composer chrome, and iOS hit-testing  
Non-goal: Desktop workspace layout, CM6 IME/caret (see
[`web/src/mdlive/PITFALLS.md`](../web/src/mdlive/PITFALLS.md)), or Provider
package UI

This document is the strategy that later Mobile motion and Transcript work
must follow. It records what survived device review, not every attempt.

## 1. Product model

Mobile is a single-task product. The workspace peeks as an opaque page. A
horizontal swipe reveals Sessions on the left or Review on the right. Agent
and Review are adjacent product pages, not two layouts of one page.

Obsidian Mobile is the existence proof for **finger-tracking** and **settle
feel**. It is not a license to copy every visual choice. Cowboy's Sessions
rail travels with the peek (`offset - width`) because a pinned rail felt
frozen under the page. Do not re-pin the rail to "match Obsidian" without
an explicit product request.

```text
gesture root (shell)
├── rail (Sessions or Review)     complementary translate
├── mask                          same peek translate
└── peek
    ├── transcript surface        peek translate
    ├── frost slab                peek translate
    ├── composer + navbar         peek translate (own layers)
    └── dim (paint only)          peek translate
```

## 2. Invariants

1. Drag is 1:1 `translate3d`. Do not interpolate finger tracking with CSS
   `transition` on `transform`.
2. Release is compositor settle: `cubic-bezier(0.32, 0.72, 0, 1)`, about
   260–360 ms. Do not run a JS spring on every frame of the settle.
3. The peek is full size and unscaled. Dim the workspace; do not `scale()`
   it.
4. iOS pins bottom chrome of a transformed viewport-sized box. Composer and
   navbar are siblings of the transcript surface and receive the same peek
   matrix. Do not put them inside a full-screen transformed ancestor.
5. Transcript scroll is `column-reverse` and must never be JS-virtualized.
   Recycle by replacing older **mounted** rows with a measured spacer while
   the reader follows the live edge.
6. Flatten compositor work **before** the first peek translate (prepare at
   ~2 px). Do not add or remove `backdrop-filter` while the finger is
   moving.
7. Chrome hit-testing and chrome paint are different layers. Never resize a
   painted veil after settle to fix taps.
8. Chrome Blink on the Mac debug profile cannot prove iOS pin, IME, or
   visual-viewport bugs. Ship after a PWA Update and physical-device check
   for those classes.

## 3. Drawer motion

### Finger tracking

`web/src/obsidianDrawerGesture.ts` is the recognizer. The workspace claims a
one-finger swipe when `|dx| > |dy|` past two CSS pixels, then writes
`translate3d` every `touchmove`. Velocity is the last 100 ms of samples.
Release: a flick (≥ 0.3 px/ms) wins, otherwise the nearer rest state
(50%). Overscroll uses the UIScrollView rubber-band, not a linear 0.18
scale.

The Sessions rail keeps an 11 px slop so a row tap is not a close. That
slop is Cowboy-specific.

Do not restore the old 4–12 px lock plus 1.15 axis ratio plus 34/66
magnetic thresholds. Those made the page feel late and sticky.

`web/src/mobileSpatialDrawer.ts` owns prepare, lock, 1:1 write, settle, and
follower transforms. AppBar/nav CSS must not include `transform` in a
standing transition; that interpolates the follower and the bottom trails
the peek by a half-beat.

### Rail and peek

`mobileDrawerRailOffset(offset, width)` is `offset - width`. Closed rail
sits off-screen; open rail meets the peek at 0. The peek uses `offset`.
Sharing one matrix made the list ride with the page.

### Followers

iOS keeps `position:absolute; bottom` chrome of a transformed full-screen
box glued to the visual viewport. Give composer, navbar, dim, and frost
their own `translate3d` with the peek matrix. Wrappers that span the
viewport stay `pointer-events: none` so taps reach the rail.

Promote `will-change: transform` only during direct manipulation. Leave it
on at rest and iOS treats the bottom chrome as a viewport-attached layer.

### Dim

Obsidian recedes the workspace with a join-to-edge gradient, not a flat
22% black veil. The painted dim stays full-size and follows the peek. Its
opacity is progress 0–1; the gradient carries the shade.

After settle, do **not** rewrite the dim to `left: drawer-width` and
`transform: none`. That restretches the gradient and the black band jumps
once. Close-taps live on a separate `[data-mobile-drawer-close]` layer
that covers only the peek in layout.

### Status strip

The top safe-area strip uses solid `background.default` — the same token as
the Sessions rail and the page. Frost on that strip returns after swipe
flatten and reads as a blurry band over the rail. Do not put
`backdrop-filter` back on the status strip.

### Composer frost

Resting composer + navbar sit on one `frostedChrome` slab
(`web/src/frostedGlass.ts`). Prepare strips `backdrop-filter` before the
first translate (`mobileCompositorFlattenSx`). Keyboard focus hides the
slab and uses opaque paper so CM6 cannot sample the transcript.

## 4. Transcript budget

A long transcript is janky because every settled row has `contain: layout
paint`, so a swipe relocates N compositor tiles. Freezing the store is not
enough: the DOM is still there.

Two budgets, two jobs:

| Budget | Module | Job |
|---|---|---|
| Event tail | `store.releaseFollowedHistory` | Drop deep ACP envelopes while following; `loadOlder` pages them back |
| Mounted rows | `transcriptLiveWindow.ts` | Keep the newest 20 **rendered** rows; older mounted rows become one spacer |

The live window only runs when the reader follows an overflowing tail.
Heights come from the last layout of those rows, so the spacer matches
what was just on screen. `column-reverse` anchors the live edge at the
bottom; collapsing the visual top does not jump the page.

Do not bring back a JavaScript virtualizer. Unmounting variable-height
rows breaks the iOS column-reverse anchor and drops local tool-card
state. Do not use `content-visibility` on rows inside that scroller:
WebKit can keep the intrinsic height and skip paint, leaving a hole.

During a drawer swipe, flatten also sets `contain: none` on
`[data-mobile-drawer-surface] [data-key]` so the peek is one tile.

## 5. Hit testing and chrome freeze

Flatten exists to make a swipe cheap. It must not disable the rail.

- Scope overflow freeze, animation pause, and paint-contain to
  `[data-mobile-drawer-surface]`. The session list is also marked
  `data-mobile-overflow-layer`; a global selector freezes row taps and
  busy spinners while the drawer is open.
- Full-width transparent wrappers (`pointer-events: none`) let row taps
  reach the rail. Sliding pieces set `pointer-events: auto`.
- Menus that can unmount their button use tap coordinates
  (`anchorPosition`), not a live node.
- Session pick closes the drawer by writing the open ref first, then
  calling `settle(false)`. Do not rebind the controller on `activeId`.

## 6. Do not retry

These already failed or were rejected:

- Scale the peek, or a heavy black veil, to fake depth.
- Pin the Sessions rail because Obsidian pins its sidebar.
- `position:absolute; inset:0` on the sliding page (iOS pins the footer).
- `transform: none !important` on session-nav/footer (detaches iOS
  layers).
- Tray-only image paste as a caret workaround. PITFALLS #69 stays OPEN.
- html2canvas or a fake bitmap as the first swipe optimization. Recycle
  and flatten first.
- U+200B / beforeinput / drawSelection retries for the iOS image caret.

## 7. Verification

- Unit/source tests lock the contracts in `obsidianDrawerGesture`,
  `transcriptLiveWindow`, `mobilePresentationMotion`, and
  `mobileSpatialDrawer`.
- Chrome CDP on `https://cowboy.stormbird.xyz/` with an iPhone UA can
  prove 1:1 matrices, hit targets, and gradient CSS. It cannot prove iOS
  visual-viewport pin, frost hitch, or keyboard flash.
- After `cowboy-web-activate`, bump `web/public/sw.js` `VERSION` and tap
  PWA **Update**. A WS reconnect keeps stale JS.

## Code map

| Concern | Owner |
|---|---|
| Swipe claim, flick, rubber | `web/src/obsidianDrawerGesture.ts` |
| 1:1 write, followers, settle | `web/src/mobileSpatialDrawer.ts` |
| Rail offset, dim progress, settle curve | `web/src/mobileDrawerMotion.ts` |
| Prepare flatten, rail hit, close layer | `web/src/mobilePresentationMotion.ts` |
| Live-row recycle | `web/src/transcriptLiveWindow.ts` |
| Column-reverse transcript | `web/src/Transcript.tsx` |
| Event-tail recycle | `store.releaseFollowedHistory` |
| Resting frost | `web/src/frostedGlass.ts` `frostedChrome` |
| Keyboard inset clamp | `web/src/keyboardGeometry.ts` |
| Shell composition | `web/src/App.tsx`, `ReviewDrawerShell.tsx` |
