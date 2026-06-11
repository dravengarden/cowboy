# mdlive — upstream sync procedure

This package is a **vendored fork** of an MIT engine. Syncing upstream bug fixes
must be a recorded, repeatable procedure — not archaeology. Read this before
touching any vendored file.

## Provenance

- Upstream: **[`github.com/kenforthewin/atomic-editor`](https://github.com/kenforthewin/atomic-editor)** (MIT).
- Baseline commit (this vendoring): **`eba2066`** — 2026-06-06, npm `v0.4.3`.
- Every vendored file carries the banner
  `// vendored from kenforthewin/atomic-editor@eba2066, MIT — see ./SYNC.md`.
- `LICENSE` here is upstream's MIT, verbatim (notice retention is required).

## Vendored files (upstream `src/…` → here `web/src/mdlive/…`)

| Upstream path | Here | Note |
|---|---|---|
| `src/inline-preview.ts` | `inline-preview.ts` | the core live-preview decorations |
| `src/tree-progress.ts` | `tree-progress.ts` | inline-preview's only sibling dep (bg-parse progress) |
| `src/atomic-theme.ts` | `atomic-theme.ts` | `atomicEditorTheme` + `atomicMarkdownSyntax` |
| `src/edit-helpers.ts` | `edit-helpers.ts` | `extendEmphasisPair` / `autoCloseCodeFence` |
| `src/styles/inline-preview.css` | `styles/inline-preview.css` | hide/reveal + layout CSS (kept verbatim; dead table/wiki rules are harmless) |

`index.ts` is cowboy-authored (not upstream — upstream ships a React wrapper we
don't use); it is the only public entry point.

## Excluded files (and why)

| Upstream file | Why excluded (v1) |
|---|---|
| `src/table-widget.ts` | **The ONLY contenteditable surface — the entire IME risk lives here.** A chat composer doesn't need tables. Excluding it keeps mdlive a pure CM6 text layer. |
| `src/wiki-links.ts` | Obsidian `[[…]]` — not needed for chat. |
| `src/image-blocks.ts` | Inline image widgets — deferred to v2. |
| `src/AtomicCodeMirrorEditor.tsx` | React wrapper; cowboy mounts CM6 via `@uiw/react-codemirror`. |
| `src/code-languages.ts` | Per-fence grammar catalog; cowboy passes `codeLanguages: []` (no embedded-code highlighting in v1). |

`inline-preview.ts` references tables/images only in **comments** (around
lines `:547` / `:571` of the baseline), never as imports — so excluding them
needs no code edit.

## Local modifications log

Keep edits minimal and `// LOCAL:`-tagged so the upstream diff stays small and
re-syncable. As of the `eba2066` vendoring, the local edits are:

1. **Per-file banner** (2 lines) at the top of each vendored file.
2. **`// @ts-nocheck` block** (3 lines, after the banner) on every vendored
   `*.ts`. Reason: cowboy's tsconfig is stricter than upstream's
   (`noUncheckedIndexedAccess`), which flags upstream's index access as ~16
   strict-null "errors" that are pure noise (runtime is unaffected). We do not
   re-typecheck third-party code we don't own. **Re-apply this on every sync.**
3. **List-marker active-line reveal** (`inline-preview.ts`, the `ListMark` branch
   — task + plain-bullet paths — and the `TaskMarker` branch). Upstream hid `- `,
   rendered the GFM checkbox, and rendered the `•` bullet widget UNCONDITIONALLY,
   so a task/bullet NEVER revealed its markdown source when the cursor was on its
   line — unlike every other marker (HeaderMark, emphasis, quote, …) which the
   engine reveals on the active line. That broke Obsidian parity (a task shows
   `• [ ]` raw while you edit/backspace it, `☐` only away; a bullet shows `- ` raw
   on the active line, `•` away). The LOCAL edits gate each on
   `!activeLines.has(line)`: inactive → hide `- ` / render `☐` / render `•`;
   active → render the bullet / leave `[ ]` raw / leave `- ` raw. The done-line
   strike is likewise inactive-only so an edited `[x]` reveals clean. On re-sync,
   re-apply over upstream's three unconditional pushes.

4. **Highlight node classes** (`inline-preview.ts`): `'HighlightMark'` added to
   `HIDEABLE_SYNTAX` and `Highlight: 'cm-atomic-highlight'` to
   `INLINE_MARK_CLASS`. cowboy adds a `==text==` highlight via a `@lezer/markdown`
   extension (`web/src/composerHighlight.ts`); these two entries let the generic
   mark hide/reveal + content-class machinery render it (marker hidden inactive /
   revealed active, content highlighted) with zero new logic. The colour is in
   cowboy's `cmTheme.ts` (`.cm-atomic-highlight`), not the vendored CSS. On
   re-sync, re-add both entries.

The CSS contents are unmodified.

## Sync workflow (run this to port an upstream fix)

```sh
# 1. Clone upstream and note the new tag/commit (e.g. v0.5.x → <new>).
git clone https://github.com/kenforthewin/atomic-editor
git -C atomic-editor log --oneline -5

# 2. Diff ONLY the vendored files between the baseline and the new commit:
git -C atomic-editor diff eba2066..<new> -- \
  src/inline-preview.ts src/tree-progress.ts src/atomic-theme.ts \
  src/edit-helpers.ts src/styles/inline-preview.css

# 3. Read the diff; apply the relevant fixes into web/src/mdlive/*, RE-APPLYING
#    the two LOCAL edits above (banner + @ts-nocheck) where they overlap.

# 4. Bump the baseline in this file + every per-file banner: eba2066 → <new>.

# 5. Re-run the VFC gates, VFC-0 (IME) FIRST, then VFC-1 / VFC-4 / VFC-6.
#    (Desktop: chrome-debug-bridge. Mobile: ios-sim MCP.)
```

If upstream adds a new sibling dep that `inline-preview.ts` imports, vendor it
too (and list it above); if it starts importing a previously-excluded file
(tables/images), that's a v2 decision — do NOT pull contenteditable in without
re-clearing VFC-0.
