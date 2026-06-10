# mdlive

Obsidian-style **markdown live-preview** for CodeMirror 6. The literal document
stays standard GFM text; formatting renders inline while editing, and the
cursor's line(s) reveal the raw markers. Vendored from the MIT
[`atomic-editor`](https://github.com/kenforthewin/atomic-editor) — see
[`SYNC.md`](./SYNC.md) for provenance + the upstream-sync procedure, and
[`LICENSE`](./LICENSE) for the MIT notice.

This is an **app-agnostic** CM6 extension package. It lives in cowboy's tree for
now but has no cowboy dependencies, so it can graduate to `@shared-utils/ui`
unchanged.

> ⚠️ **Before touching the composer editor, read [`PITFALLS.md`](./PITFALLS.md).**
> The CM6 markdown editor on iOS WebKit has a field of *coupled* pitfalls (IME ↔
> caret ↔ the native paste menu ↔ widget render ↔ cowboy's compositing hack).
> PITFALLS.md is the map + the Obsidian-alignment contract + the "do NOT
> whack-a-mole" rule + the full verification matrix to run after any change.

## Public API (`./index.ts`)

All factories return CM6 `Extension`s.

| Export | What |
|---|---|
| `inlinePreview(config?)` | The live-preview decorations + the markdown-aware Enter/keymap. `config.onLinkClick?: (url) => void` — called on a plain click of a rendered link; defaults to `window.open(url, '_blank', …)`. In a Tauri/Electron shell, pass an opener that routes through the host. |
| `atomicEditorTheme` | The base editor theme (transparent, reading layout). |
| `atomicMarkdownSyntax` | Syntax highlighting for markdown tokens (`syntaxHighlighting(atomicMarkdownHighlight)`). |
| `atomicMarkdownHighlight` | The raw `HighlightStyle` (compose your own if needed). |
| `extendEmphasisPair` | Optional: typing inside `**`/`*`/`` ` `` extends the pair (toolbar bold/wrap). |
| `autoCloseCodeFence`, `autoCloseCodeFenceInput` | Optional: auto-close ` ``` ` fences. |

### Minimal mount (`@uiw/react-codemirror`)

```tsx
import CodeMirror from "@uiw/react-codemirror";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { inlinePreview, atomicEditorTheme, atomicMarkdownSyntax } from "./mdlive";
import "./mdlive/styles/inline-preview.css"; // import the CSS once, app-wide

const extensions = [
  markdown({ base: markdownLanguage, codeLanguages: [] }), // [] = no embedded-code grammars (v1)
  atomicEditorTheme,
  atomicMarkdownSyntax,
  inlinePreview({ onLinkClick: (url) => openExternal(url) }),
];

<CodeMirror value={md} onChange={setMd} extensions={extensions} basicSetup={false} theme="none" />;
```

The value in/out is always the literal markdown string — there is no
serialization round-trip, so switching hosts (collapsed ↔ expanded ↔ fullscreen)
never changes the text.

## Invariants — DO NOT break these

1. **Active-line reveal**: `shouldHide = !activeLines.has(lineNum)` — the cursor/
   selection line carries NO hide/replace decorations, so the raw characters are
   present where you type. This is what keeps **IME composition** (Chinese pinyin)
   and **vim column motions** working. Never make it hide the active line.
2. **No contenteditable surface**: this build excludes `table-widget.ts` (the only
   contenteditable widget, and the only place that did mid-composition
   `commit()` rebuilds). Keep mdlive a pure CM6 text layer — do not vendor tables
   without re-clearing the IME gate.
3. **`viewportChanged` is intentionally NOT a decoration-rebuild trigger** — this
   is the iOS momentum-scroll fix. Do not add it to the rebuild conditions.
4. **One `@codemirror/*` instance, `@codemirror/view` ≥ 6.5**: a second
   `@codemirror/state`/`view` makes the extensions silently no-op (CM6's
   singleton rule), and < 6.5 reintroduces an IME composition bug.

## Excluded in v1

Tables, wiki-links (`[[…]]`), inline image blocks, and embedded fenced-code
language highlighting. See [`SYNC.md`](./SYNC.md) → "Excluded files".
