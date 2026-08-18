# cowboy/web — agent notes

The cowboy PWA frontend (React + MUI + CodeMirror 6, Deno + Vite). Read this
before editing, then the topic docs it routes to.

## ⚠️ The markdown composer editor — read PITFALLS.md FIRST

The compose surfaces (`src/Composer.tsx`, `src/ComposerEditor.tsx`,
`src/FullscreenComposer.tsx`, `src/composerExtensions.ts`, the vendored
`src/mdlive/*`) run a CM6 + Obsidian-style live-preview editor. On **iOS
WebKit** its behaviours are **coupled** — IME composition, the caret, the
native long-press paste/select menu, widget rendering, and cowboy's historical
compositing workarounds all interact. A change that "fixes" one
routinely breaks another.

**Hard rules (the user's standing instruction — do not deviate):**

1. **Read [`src/mdlive/PITFALLS.md`](src/mdlive/PITFALLS.md) before any composer
   change.** It is the authoritative map: the extension inventory (what's in and
   WHY), every known iOS pitfall (symptom → cause → fix), and the verification
   matrix.
2. **Align with Obsidian — mandatory.** Obsidian mobile is the existence proof
   that this CM6 stack works on iOS WebKit. When unsure, do what Obsidian does
   (it's why we vendored `atomic-editor`). Diverging needs a written reason in
   PITFALLS.md.
3. **Do NOT whack-a-mole.** Never toggle one CM6 extension on/off to chase a
   single symptom. Change one thing, then re-verify the WHOLE iOS matrix (caret +
   IME + paste + render + expand/collapse) on the **iOS Simulator or device** —
   the Chrome bridge cannot reproduce iOS IME / caret / paste-menu.
4. **Record every decision in PITFALLS.md** so the next agent doesn't re-discover
   it.
5. **OPEN TODO:** physical iPhone Return after a pasted image still fails
   (painted UIKit caret vs CM6). 2026-08-15 user: same shape in Obsidian;
   likely WeType (first Return on native Pinyin, first few on WeChat IME).
   Full ledger, failed attempts, and what not to retry:
   [`src/mdlive/PITFALLS.md`](src/mdlive/PITFALLS.md) pitfall **#69**. Do
   not claim this fixed. Do not re-ship Obsidian token reveal.

## Editing discipline (project-specific)

- **NEVER `deno fmt` `src/App.tsx`** — it is a pre-existing 4-space outlier
  (rest of `src/` is 2-space); a format would wholesale-reflow ~3700 lines.
  Match 4-space when editing it.
- A fresh worktree lacks `src/_shell` — symlink it to the shared UI package:
  `ln -s /home/draven/projects/shared-utils/packages/ui src/_shell` (gitignored).
- Quality gate before commit: `deno check` + `oxlint` (the cowboy web gate). Do
  not run repo-wide `deno fmt`.

## Mobile drawers, Code Review, and long transcripts

**Hard rule: a swipe that drops frames is a product bug.** Left/right
drawer and Agent↔Review pager must stay 1:1 on iPhone whether the peek
shows Transcript, README, or a wrap-off CodeMirror justfile. This is a
core Mobile requirement, not a later polish pass.

Before changing Sessions/Review swipe, peek chrome (Review header or
bottom nav, Agent composer/navbar), composer frost, Transcript paint
cost, or Code Review's editor/tree on Mobile, read
[`docs/mobile-spatial-presentation.md`](../docs/mobile-spatial-presentation.md)
§2.1. Those regions inherit the peek compositor: a header toggle or
MUI `Switch` is a swipe-path change. Nested `transform` / drop shadow
inside `translate3d` reassembles tiles every frame. Keep selected chrome
paint-only. The rest of the contract: jank-free swipe, 1:1
`translate3d`, complementary rail, follower layers for iOS pin, live-row
recycle without a JS virtualizer, a standing peek compositor layer (arm
on finger-down; first tracking frame only writes transform), wrap-on
Review source as a workspace swipe (live CodeMirror, no sticky gutters,
no touch-scroll tile; wrap-off keeps the native X bar), and paint/hit
split for the dim. Intermittent frame drops are assemble-at-prepare, not
tracking math — do not retune the cubic or `setState` on `touchstart` to
chase them. Do not hide or snapshot the editor, re-pin the rail, scale
the peek, or freeze the session list to make a swipe cheaper.

## Desktop and mobile are separate products

- Mobile is touch-first: single-task focus, large targets, progressive
  disclosure, safe system-keyboard behaviour, and native gestures.
- Desktop is efficiency-first: keyboard-first operation, high information
  density, full use of available space, visible controls, composable commands,
  and parallel context. Minimalism must never hide useful capability or add
  interaction layers merely to make the surface look clean.
- A user who knows basic Vim must be productive without memorising Cowboy-only
  shortcuts. Prefer standard Vim motion, convenient bare contextual keys, the
  platform workspace prefix (`Cmd+K` on macOS, `Alt+K` elsewhere), contextual
  status-line hints, and a searchable `Mod+Shift+P` command palette. The prefix
  works in Vim Insert/Normal/Visual and native inputs but never during IME
  composition or through an exclusive modal/menu. Global bare product letters
  are forbidden. Chrome non-conflict is a core requirement; every new binding
  follows the executable policy in
  [`src/desktop/FOCUS.md`](src/desktop/FOCUS.md). Do not add a Space leader
  layer: it proved slower than direct, visible commands here.
- Prefer MUI's native component semantics and composition for Desktop UI:
  AppBar/Toolbar, Tabs, Menu, List, Select, Dialog, Tooltip, theme tokens, and
  `sx`. Build custom primitives only where MUI has no suitable interaction
  model, such as pane splitters and the Vim status line.
- Desktop must never render a page-wide Vimium-style target-hint overlay or
  reserve bare `f` for one. It duplicates Cowboy's own navigation model and
  obscures the working surface. Keep actions reachable through native focus,
  direct Vim motions, visible contextual shortcuts, and the Command Palette.
- Share protocol, stores, API clients, domain logic, attachments, and markdown
  machinery. Desktop and mobile may intentionally duplicate layout, component,
  and interaction code so either product can evolve without responsive-UI
  compromises.
- Never decide whether Desktop controls are visible from a mobile breakpoint or
  from the fact that a pane is in split mode. Use the pane's actual available
  space and the Desktop workflow's efficiency needs.
- Confirmation modals (Clear, Compact, Stop, Reload, discard, delete, update,
  Provider auth/uninstall) use `ConfirmSheet`. Mobile and tablet always get
  the compact Obsidian-style inset card (`ObsidianSheet`); Desktop keeps a
  centered dialog. Do not add a raw MUI `Dialog` for a phone-facing confirm.
  Cover/workbench sheets (Settings, New Session) stay on DetentSheet.

## Product login gate

`ProductAuthGate` wraps `DesktopApp` / `MobileApp` in `src/main.tsx`. Keep it
out of `src/App.tsx`. `src/auth/*` must not import `src/store.ts` —
`subscribe()` opens `/ws` on the first listener, and the logged-out branch must
never construct a WebSocket.

`GET /api/auth/status` decides the surface: HTTP 200 + `me` mounts the apps;
HTTP 200 + missing `me` is login (register only when
`registration.accepts_registration`; invite field when `mode === "token"`);
HTTP 404/501, 200 HTML, or a body without the registration shape is
“controller too old / activating”, never login-forever;
network / 5xx retries with banner backoff and must not clear the cookie.
Once the apps are mounted, a later 5xx/404/501 does not unmount them.
Sign-out (and 200 without/`me` change) emits `cowboy:product-sign-out` so
`store.ts` closes `/ws` without reconnecting, then reloads. `sw.js` must not
cache `/api/auth/*`. Logout and a `me` account change delete `HISTORY_CACHE`.

## Deploy (web changes reach the installed PWA only via a SW version bump)

1. Bump `web/public/sw.js` → `const VERSION = "cowboy-vNN"` (the foreground
   update-check only fires when this string changes — it triggers the auto-reload
   onto the fresh bundle).
2. `deno check` + `oxlint`, commit on the branch.
3. Build picks up new npm deps via a deps-FOD; if you ADDED deps, capture the new
   `depsHash` with `nix build .#cowboy-web --option sandbox false` (DNS fails
   under the nix sandbox on hawk — see columbus memory `deno-vite-fod-dns-sandbox`).
4. A web-only host switch atomically retargets `/run/cowboy-web`; it does not
   restart Cowboy, Machine, or session workers. Verify the new `/version` and
   `sw.js`; the PWA foreground update check performs the reload.
5. Desktop also has a pre-module recovery guard in `index.html`. Keep it before
   `/src/main.tsx`: it is the only layer that can recover an installed Desktop
   window when an old hashed entry or lazy chunk fails before React mounts.
   This guard must stay Desktop-only; Mobile owns its established PWA path.
