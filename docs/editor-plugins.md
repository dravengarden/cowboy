# Editor plugins and Desktop editing parity

Every Draft and Session prompt uses one editor component
(`PlatformComposerEditor`) and one extension host. This document covers the
installable editor plugin mechanism added on top of the built-in Templates and
Outline extensions, and the Desktop editing comparison with Obsidian.

## Installable editor plugins

Obsidian is the reference architecture: a plugin is a manifest plus one
JavaScript entry, it registers commands, ribbon/toolbar actions, views and
settings in `onload`, and every registration is released on unload. Cowboy does
not run Obsidian plugins natively; it has its own small API.

### Package and manifest

A package is one JSON file (`.cowboy-plugin`, format
`cowboy-editor-plugin/1`) holding the manifest, the `main.js` text and a
`sha256-` digest over the canonical manifest and the code. Pack a plugin
directory with:

```bash
just editor-plugin-pack examples/editor-plugins/text-tools text-tools.cowboy-plugin
```

The manifest is closed (unknown fields, permissions, setting types and icons
are rejected) so a newer package can never gain authority silently on an older
host:

| Field | Meaning |
| --- | --- |
| `id`, `name`, `version`, `description`, `author` | Identity; `version` is `MAJOR.MINOR.PATCH`; the `cowboy-` id prefix is reserved for built-ins |
| `api: { major, minor }` | Editor plugin API the code targets. The host (currently `1.0`) accepts the same major with an equal or older minor; anything else is shown as incompatible before install |
| `permissions` | `editor:read` (text and selection of the editor a command or panel runs in) and `editor:write` (replace that selection) |
| `contexts`, `surfaces` | `document` (Draft) / `session`; `desktop` / `touch` |
| `settings` | Declarative `boolean`, `string`, `number` and `select` settings that the host renders and validates |

### Authoring API

```js
definePlugin({
  async onload(ctx) {
    ctx.addCommand({ id, title, description, icon, toolbar, run(editor) });
    ctx.addPanel({ id, title, render(doc) });   // returns [{ label, detail, offset, depth }]
    ctx.settings;  ctx.onSettingsChange(fn);
    await ctx.loadData();  await ctx.saveData(json);   // ≤ 64 KiB
    ctx.notice(text);
  },
  onunload() {},
});
```

`editor.read()` returns `{ text, selection, revision }` with `editor:read`.
`await editor.replaceSelection(text)` needs `editor:write` and resolves `false`
when the user kept typing, an IME owns the editor, the command finished or the
editor was closed. Contributions are registered during `onload` only.
`icon` is one of a closed set (`text`, `sort`, `list`, `wand`, `clock`, `calc`,
`tag`, `link`). [`examples/editor-plugins/text-tools`](../examples/editor-plugins/text-tools)
uses every surface.

### Registration surfaces

One registration reaches every surface, so actions, hints and the palette
cannot disagree:

- **Command Palette** (Desktop): each command is the Desktop command
  `plugin.<plugin>.<command>`, titled `Plugin: Command`, group *Plugins*.
- **Toolbar** (Desktop): commands with `toolbar: true` appear in the Draft and
  Session prompt toolbars and execute that same registered command. Buttons
  keep focus in the editor and wrap with the existing density tiers.
- **Editor extensions → Tools** (Desktop and touch): commands and panels for
  the editor the sheet was opened from. Touch gains no new toolbar controls.

### Lifecycle

Settings → Editor extensions → *Install plugin…* reads the file, verifies its
digest and shows name, version, author, digest prefix, contexts and the exact
permissions (new permissions are flagged on upgrade). Installing:

1. stores the package for the signed-in principal on this device
   (`service:editor-plugins` in the product IndexedDB dataset; it survives
   sign-out but is not synchronized to other devices);
2. starts it in a sandbox and waits for `onload` (5 s);
3. on success records `install`/`upgrade`/`downgrade`/`reinstall` in the
   plugin history.

A failing **upgrade** restores the exact previous package, settings, data and
enabled state and records `rollback`. A failing fresh install stays installed
but off, with its error and a *Retry* action. Disable stops the sandbox;
uninstall removes code, the rollback copy, settings and data together. A plugin
that fails at startup stays off across reloads instead of crash-looping.
Lifecycle steps are serialized, and every storage write derives from the
latest committed state, so a running plugin's `saveData()` never overwrites a
concurrent lifecycle change. Unreadable storage is never treated as "nothing
installed".

### Isolation and authority

- Each plugin runs in its own Worker created inside an
  `<iframe sandbox="allow-scripts" srcdoc>` (no `allow-same-origin`). The frame
  has an opaque origin: no App cookies, storage, DOM or same-origin APIs.
- The frame's CSP is `default-src 'none'; script-src 'unsafe-inline' blob:;
  worker-src blob:` and the Worker inherits it, so `fetch`, WebSocket,
  `importScripts`, images and frames are blocked. The prelude also removes the
  network globals for clearer errors, but the CSP is the boundary: the
  acceptance probe bypasses the prelude and receives a `connect-src`
  violation. The App currently sends no CSP; a future App CSP must keep
  `worker-src blob:` for this frame.
- The plugin talks to the host only through a closed message protocol over a
  `MessagePort`. Every request is validated and bounded (contributions ≤ 64,
  replacement ≤ 1 MiB, panel ≤ 200 items, data ≤ 64 KiB).
- Each command invocation gets a token bound to one editor port and the
  snapshot taken at invocation. Writes go through the host's version-bound
  `EditorPort.replaceSelection`, which refuses when the document or selection
  changed, an IME composition owns the editor, or the editor was unmounted.
  The token dies when the command settles, times out or the plugin stops, so a
  plugin cannot keep editing a document after switching or closing it.
- Edits are ordinary CodeMirror transactions: one undo step per replacement,
  Vim and IME ownership unchanged.
- A command or panel that does not answer in time (10 s / 2 s) terminates the
  Worker by removing its frame and marks the plugin failed; the App thread
  keeps running. A thrown command error is reported and does not stop the
  plugin. Three uncaught errors after load stop it. One plugin's failure never
  affects another plugin, saving or editing.

### Relation to the Cowboy Plugin lifecycle

Machine Plugins (Providers, Code, workspace extensions) are signed Catalog
packages installed per Machine. Editor plugins are Machine-independent because
Drafts are; they reuse the lifecycle vocabulary (install, upgrade, rollback,
disable, uninstall, failure history, exact digest) but are installed by the
user on a device, like an Obsidian manual install. Publishing editor plugins
through the signed Catalog (an `editor_plugin` kind) and syncing installed
plugins across devices are not implemented.

## Desktop editing parity with Obsidian

Baseline: Obsidian Desktop **1.13.7** (public, 2026-08-12), confirmed from the
official changelog and `desktop-releases.json` on 2026-10-05. 1.14.x builds
were Catalyst (early access) at that date. Obsidian itself was not run on this
host; behaviour below comes from Obsidian's documented commands and the
vendored Obsidian-aligned editor code.

| Area | Obsidian 1.13.7 | Cowboy | Status |
| --- | --- | --- | --- |
| Shared editor | One editor for all notes | One component for Drafts and every Session editor | Same |
| Auto-pair brackets/Markdown | Obsidian closeBrackets fork | Vendored fork (`obsidianAutoPair`) | Same |
| List/quote continuation, Shift+Enter, Tab/Shift+Tab | smartIndentList | `obsidianMarkdownKeymap` | Same |
| Bold / Italic | Mod+B / Mod+I | **Mod+B / Mod+I added**, plus `␣B` / `␣I`; off macOS outside Vim Insert, Ctrl-B/Ctrl-I stay Vim keys | Same (Vim-aware) |
| Insert link | Mod+K | `␣U`; Mod+K is the workspace prefix on macOS | Intentional divergence |
| Toggle checklist | Mod+L | `␣O` list and More menu; Mod+L is the browser address bar | Intentional divergence |
| Live Preview / Source | Mod+E | `␣E`; Mod+E is reserved by Chrome/macOS | Intentional divergence |
| Rich-text paste | Auto convert HTML (on by default) | **Added on Desktop**: semantic HTML (headings, emphasis, links, lists/tasks, quotes, code, tables, remote images) becomes Markdown in one undo step; unsafe URLs and data URIs are dropped; styled-only HTML pastes plain text | Same on Desktop; touch unchanged |
| Paste as plain text | Mod+Shift+V | Browser delivers plain text only; conversion is skipped | Same |
| URL over selection | `[selection](url)` | Same | Same |
| Image paste | Saves attachment | Attachment + inline image token | Same behaviour, Cowboy storage |
| Undo continuity | Paste is its own step | `input.paste` never joins typing; plugin edits are one step | Same |
| Command palette + hotkeys | Palette, user-assignable hotkeys | Palette with every command incl. plugins; no user hotkey assignment | Gap |
| Plugins | Community/manual install | Manual package install, sandboxed | Different model (see above) |
| Long notes | Viewport rendering | Viewport rendering; 84,390-character note insert 77–90 ms in headless Firefox | Measured only in headless Firefox |

Fit: the Draft toolbar (including plugin buttons) has no overflow at
1200/16, 800/32, 640/24, 480/24 and 480/16 (window px / root font px) in the
integrated App. The Save label now joins the density tiers; before, Save and
its keycap overflowed a narrow pane at 24 px. At 320 px with a 32 px root font
the integrated Desktop sidebar leaves the editor pane about 48 px wide; that
layout limit is not solved here.

## Verification and remaining gaps

Automated (see `docs/acceptance-results/editor-plugins-2026-10-05.json`):

- Unit: manifest/digest validation, install, permissions, stale tokens after
  typing/IME/command end, upgrade rollback, hang termination persisting across
  reload, settings, uninstall, failed persistence, the example package, and
  HTML→Markdown conversion.
- `editor-plugin-sandbox` suite in pinned Firefox and Chromium: real
  iframe+Worker sandbox, opaque origin, CSP `connect-src` violation, version-
  bound edit, infinite loop terminated without blocking the App thread.
- `draft-documents` suite in pinned Firefox (light and dark): the packed example
  file installed through the actual manager UI in the integrated App, toolbar
  and palette commands, editor undo, live settings, hostile-plugin probe,
  failure isolation, toolbar fit and uninstall cleanup. In Chromium this suite
  stops at the pre-existing "Esc on the tablist closes Create" check, which
  also fails on unmodified `origin/main`.
- `desktop-composer` suite in Firefox and Chromium: Mod+B/Mod+I.

Not verified by these runs: real IME candidate selection and cancellation with
actual input methods, physical iPhone/iPad input (pitfall #69 remains open),
and Obsidian side-by-side timing. Synthetic composition events do not stand in
for a real input method.
