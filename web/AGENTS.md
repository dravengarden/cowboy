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

## Editing discipline (project-specific)

- **NEVER `deno fmt` `src/App.tsx`** — it is a pre-existing 4-space outlier
  (rest of `src/` is 2-space); a format would wholesale-reflow ~3700 lines.
  Match 4-space when editing it.
- A fresh worktree lacks `src/_shell` — symlink it to the shared UI package:
  `ln -s /home/draven/projects/shared-utils/packages/ui src/_shell` (gitignored).
- Quality gate before commit: `deno check` + `oxlint` (the cowboy web gate). Do
  not run repo-wide `deno fmt`.

## Desktop and mobile are separate products

- Mobile is touch-first: single-task focus, large targets, progressive
  disclosure, safe system-keyboard behaviour, and native gestures.
- Desktop is efficiency-first: keyboard-first operation, high information
  density, full use of available space, visible controls, composable commands,
  and parallel context. Minimalism must never hide useful capability or add
  interaction layers merely to make the surface look clean.
- A user who knows basic Vim must be productive without memorising Cowboy-only
  shortcuts. Prefer standard Vim motion, a small global Mod+letter map,
  contextual status-line hints, and a searchable command palette. Do not add a
  Space leader layer: it proved slower than direct, visible commands here.
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
