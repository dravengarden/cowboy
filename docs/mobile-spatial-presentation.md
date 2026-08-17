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
6. The peek is already one compositor layer at rest. Collapse per-row
   `contain` on `[data-mobile-drawer-surface] [data-key]` standing, and
   arm hold + peek `will-change` on finger-down. The 2 px claim only
   writes `translate3d` and freezes overflow. Do not add or remove
   `backdrop-filter` on the dedicated frost follower during a drawer
   swipe — toggling it rebuilds that layer on the first tracking frames.
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

`web/src/mobileSpatialDrawer.ts` owns arm, prepare, lock, 1:1 write, settle, and
follower transforms. Finger-down arms the peek layer (store hold + peek
`will-change`). Prepare at 2 px only claims the swipe: `transition: none`,
follower promotion, overflow flatten. AppBar/nav CSS must not include
`transform` in a standing transition; that interpolates the follower and
the bottom trails the peek by a half-beat.

Do not `setState` on transcript `touchstart` or swipe-claim. A following
reader unfollows when scroll actually leaves the live edge, not on
finger-down. `holdStorePresentation` plus a pause ref freeze the tree;
React catch-up runs after release.

### Rail and peek

`mobileDrawerRailOffset(offset, width)` is `offset - width`. Closed rail
sits off-screen; open rail meets the peek at 0. The peek uses `offset`.
Sharing one matrix made the list ride with the page.

### Followers

iOS keeps `position:absolute; bottom` chrome of a transformed full-screen
box glued to the visual viewport. Give composer, navbar, dim, and frost
their own `translate3d` with the peek matrix. Wrappers that span the
viewport stay `pointer-events: none` so taps reach the rail.

Promote `will-change: transform` on the peek and mask at finger-down, and
on composer/nav/frost only after the swipe is claimed. Leave follower
`will-change` on at rest and iOS treats the bottom chrome as a
viewport-attached layer.

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
(`web/src/frostedGlass.ts`). The slab is a dedicated follower with its
own `translate3d`. Keep its `backdrop-filter` during a drawer swipe —
stripping it at prepare was the intermittent first-frame hitch. The
product pager and detent sheet still strip frost because those surfaces
*contain* the blur. Keyboard focus hides the slab and uses opaque paper
so CM6 cannot sample the transcript.

## 4. Transcript budget

A long transcript is janky because every settled row has `contain: layout
paint`, so a swipe relocates N compositor tiles. Freezing the store is not
enough: the DOM is still there.

Two budgets, two jobs:

| Budget | Module | Job |
|---|---|---|
| Event tail | `store.releaseFollowedHistory` | Drop deep ACP envelopes while following; `loadOlder` pages them back |
| Mounted rows | `transcriptLiveWindow.ts` | Keep enough newest **rendered** rows to cover the viewport, with a 20-row floor; older mounted rows become one spacer |

The live window only runs when the reader follows an overflowing tail.
Heights come from the last layout of those rows, so the spacer matches
what was just on screen. `column-reverse` anchors the live edge at the
bottom; collapsing the visual top does not jump the page.

Do not bring back a JavaScript virtualizer. Unmounting variable-height
rows breaks the iOS column-reverse anchor and drops local tool-card
state. Do not use `content-visibility` on rows inside that scroller:
WebKit can keep the intrinsic height and skip paint, leaving a hole.

`contain: none` on `[data-mobile-drawer-surface] [data-key]` is standing
(`mobilePeekRestLayerSx`), not a prepare-time toggle. Swiping a long
transcript must not restyle N paint boundaries on the first frame.

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
- Retune settle easing, flick thresholds, or 1:1 math to fix *intermittent*
  first-frame drops. That hitch is assemble-at-prepare, not tracking.
- Strip `backdrop-filter` on the dedicated frost follower at prepare
  "because translating a blur is expensive". The toggle rebuilds the
  layer on the first tracking frames and is why silk was uneven.
- `setState` on transcript `touchstart` or swipe-claim to freeze the
  tree (`detach`, `setFollowingLive`, `setRenderPausedForScroll`). The
  React commit *is* the hitch. Use a pause ref; unfollow when
  reader-owned scroll leaves the live edge.
- Apply overflow flatten on finger-down. That steals the first vertical
  scroll pixel. Overflow freeze waits for the 2 px horizontal claim.
- Write `transform-origin`, `box-shadow`, or `setAttribute` inside
  `applySlide`. Touchmove writes only `transform` (and `transition: none`).

## 7. Uneven silk

Obsidian is already one workspace layer. Cowboy's remaining hitch is not the
1:1 tracker. **"Sometimes silky, sometimes a dropped frame"** means the first
tracking frames sometimes rebuild the compositor tree.

Open-from-rest hitching while close-while-open stays cheap is the signature:
`data-mobile-drawer-open` / `presented` already apply flatten, so the close
path is pre-assembled. The expensive open path used to do all of this on the
same frames as the first `translate3d`:

- restyle `contain` on every `[data-key]`
- add/remove `backdrop-filter` on the frost slab
- `setState` in transcript `touchstart` (`detach`, unfollow, pause render)
- `will-change`, overflow freeze, moving attr, store hold, and a custom event

Do not retune easing or claim thresholds to chase that. Split the work:

| When | Do | Do not |
|---|---|---|
| Rest | `contain: none` on peek rows (`mobilePeekRestLayerSx`); identity translate on the page | `overflow: hidden` on the scroller; `will-change` on bottom chrome; strip a dedicated frost follower |
| Finger-down | `holdStorePresentation`; `will-change` on peek + mask | React commit; unfollow; overflow freeze |
| 2 px claim | `transition: none`; promote followers; freeze overflow | Restyle N rows; toggle frost |
| Each `touchmove` | `transform` (+ `transition: none`) | `setAttribute`, `setState`, `transform-origin`, `box-shadow` |

`holdStorePresentation` plus `renderPausedRef` freeze the tree. Catch-up
React after release. A following reader unfollows when reader-owned scroll
leaves the live edge — a Sessions swipe starts as a transcript `touchstart`.

Dedicated frost (`frostedChrome` on its own follower) keeps its filter for
the whole drawer gesture. Product pager and detent sheet still strip frost
because those surfaces *contain* the blur.

## 8. Verification

- Unit/source tests lock the contracts in `obsidianDrawerGesture`,
  `transcriptLiveWindow`, `mobilePresentationMotion`, and
  `mobileSpatialDrawer`.
- Chrome CDP on `https://cowboy.stormbird.xyz/` with an iPhone UA
  (`platform: iPhone`, `maxTouchPoints > 1`, `pointer: coarse`, width
  under 600 or Cowboy stays Desktop/tablet) can prove 1:1 matrices, a
  shared peek/frost/composer translate, standing `contain: none`, frost
  staying on during a drawer swipe, hit targets, and gradient CSS. It
  cannot prove iOS visual-viewport pin, 60 fps, or keyboard flash.
- After `cowboy-web-activate`, bump `web/public/sw.js` `VERSION` and tap
  PWA **Update**. A WS reconnect keeps stale JS.

## 9. Code map

| Concern | Owner |
|---|---|
| Swipe claim, flick, rubber | `web/src/obsidianDrawerGesture.ts` |
| 1:1 write, followers, settle | `web/src/mobileSpatialDrawer.ts` |
| Rail offset, dim progress, settle curve | `web/src/mobileDrawerMotion.ts` |
| Standing peek layer, prepare flatten, rail hit, close layer | `web/src/mobilePresentationMotion.ts` |
| Live-row recycle | `web/src/transcriptLiveWindow.ts` |
| Column-reverse transcript; pause ref (no `setState` on finger-down) | `web/src/Transcript.tsx` |
| Event-tail recycle | `store.releaseFollowedHistory` |
| Resting frost | `web/src/frostedGlass.ts` `frostedChrome` |
| Keyboard inset clamp | `web/src/keyboardGeometry.ts` |
| Shell composition | `web/src/App.tsx`, `ReviewDrawerShell.tsx` |
