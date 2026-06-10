# mdlive — iOS / CM6 pitfalls & the Obsidian-alignment contract

> **READ THIS BEFORE TOUCHING the composer editor (`ComposerEditor.tsx`,
> `composerExtensions.ts`, `FullscreenComposer.tsx`, `ComposerTextarea.tsx`, the
> vendored `mdlive/*`).** The CM6 markdown editor on **iOS WebKit** has a dense
> field of *interacting* pitfalls — IME composition, the caret, the native
> long-press paste/select menu, widget rendering, and cowboy's own
> compositing-layer hacks all fight each other. A change that "fixes" one of
> these routinely breaks another. This file is the map so we stop re-discovering
> them.

## The cardinal rule — do NOT whack-a-mole

These bugs are **coupled on iOS**. Toggling one CM6 extension on/off to chase a
single symptom is how we shipped a string of regressions (drawSelection fixed
the caret but was blamed for paste; removing it "fixed" paste but the caret
blinks out; …). Instead:

1. **Align with Obsidian, deliberately.** Obsidian mobile is the *existence
   proof*: the same CM6 stack works on the same iOS WebKit. When in doubt, do
   what Obsidian's editor does (it's why we vendored `atomic-editor`, an
   Obsidian-style engine). Diverging from it needs a written reason here.
2. **Change one thing, then verify the WHOLE iOS matrix** (see §"Verification
   matrix") — not just the symptom you were chasing. Caret + IME + paste +
   render + expand/collapse must ALL still pass.
3. **Record the decision here.** Every extension we add/drop, and every iOS
   workaround, gets a line in §"Extension inventory" / §"Known pitfalls" with
   the symptom → cause → fix → *and which other thing it risks*.
4. **Prefer real-device / Simulator verification over emulated Chrome.** The
   Chrome bridge (touch-emulation) catches layout/render bugs but CANNOT
   reproduce iOS WebKit IME, the native caret, or the long-press menu. Those are
   the ones that bite. Use the iOS Simulator (AXe + simctl) or the user's device.

## Extension inventory (`composerExtensions.ts`) — what's in and WHY

The shared `livePreviewExtensions()` is a deliberate port of atomic-editor's
(Obsidian-style) CM6 composition. Keep it aligned with Obsidian unless a row
here says otherwise.

| Extension | In? | Why / which pitfall |
|---|---|---|
| `markdown({base: markdownLanguage, codeLanguages: []})` | ✅ | the syntax tree the engine reads. `codeLanguages: []` = no embedded-code grammars (keep deps small). |
| `inlinePreview()` + `atomicEditorTheme` + `atomicMarkdownSyntax` | ✅ | the live-preview decorations + theme + highlight (the whole point). |
| `closeBrackets()` + `markdownLanguage.data.of({closeBrackets:{brackets:[… * _ ` ]}})` | ✅ | Obsidian-style auto-pairing incl. emphasis delimiters. |
| `extendEmphasisPair` | ✅ | grow `*|*` → `**|**` as you type. |
| `autoCloseCodeFence` | ✅ | auto-close ``` fences. |
| `Prec.high(keymap.of(closeBracketsKeymap))` | ✅ | **Backspace deletes an empty pair as a unit** (`*|*`→empty). MUST be `Prec.high` — else cowboy's `defaultKeymap` `deleteCharBackward` wins and orphans the closer. |
| `markdownKeymap` | ✅ | markdown keybindings (list continuation is owned by inlinePreview's `Prec.highest` Enter, so markdownKeymap's Enter is beaten — fine). |
| `indentOnInput()` | ✅ | auto-indent. |
| `drawSelection()` | ✅ (Obsidian) | **CM draws the caret/selection itself.** This is how Obsidian keeps the caret visible on iOS — the *native* caret BLINKS OUT under cowboy's `translateZ(0)` scroller compositing layer ("光标经常消失，尤其上下移动时"). See pitfall #2. It only OVERLAYS the native selection (does not remove it), so the iOS paste menu is unaffected — see pitfall #3. |
| `dropCursor()` | ✅ | drag-drop caret (Obsidian has it). |
| `highlightActiveLine()` | ✅ | active-line bg (atomic-theme styles `.cm-activeLine`). |
| `search({top:true})` + `searchKeymap` | ✅ | find-in-doc (fullscreen long-form). Needs `@codemirror/search`. |
| `EditorView.theme({".cm-widgetBuffer":{visibility:"hidden"}})` | ✅ | hide the iOS broken-image dot (pitfall #4). |
| `indentWithTab` | ✅ | Tab indent. |
| `rectangularSelection()` / `allowMultipleSelections` | ❌ | multi-cursor; desktop-only nicety, needs drawSelection to render. Re-add desktop-only if ever wanted. |
| `history()` / `historyKeymap` / `defaultKeymap` | ❌ | **cowboy's `ComposerEditor` base already provides these.** A 2nd `history()` SPLITS UNDO. Never add here. |
| `table-widget` / `image-blocks` / `wiki-links` | ❌ | the only **contenteditable** widgets = the IME landmine; and cowboy has no notes vault for `[[wiki-links]]`. Out of v1. See SYNC.md. |
| `highlightSpecialChars()` | ❌ | renders dots for special chars — noise in a chat box. |
| `initialRevealField` | ❌ | React-wrapper-only (reveal-on-open); no cowboy use case. |

## Known pitfalls (symptom → cause → fix → status)

1. **Chinese pinyin IME drops/garbles on a marker line.** The original reason
   the mobile composer used a native `<textarea>`. CM6 contenteditable + IME on
   iOS is the hard constraint. **Mitigations that make it safe:** (a) the engine
   reveals raw markers on the *active line* (`shouldHide = !activeLines.has(lineNum)`)
   so you never compose inside a hidden range; (b) NO contenteditable widgets
   (tables/images excluded); (c) `@codemirror/view ≥ 6.5` (composition fix);
   (d) cowboy's `compositionstart/end` handler in `ComposerEditor.tsx` drops the
   `translateZ(0)` scroller transform during composition. **Status: verified on
   the Simulator (imeSetComposition) + desktop. Re-verify on EVERY editor change.**

2. **Caret "经常消失" (blinks out / invisible), esp. when moving up/down.** iOS
   WebKit hides the *native* caret when the editor's scroller is a promoted
   compositing layer (cowboy's `cmTheme` uses `translateZ(0)` to force repaint of
   the fixed-body content); vertical caret motion is the worst case. **Fix =
   `drawSelection()`** — CM draws its own `.cm-cursor` positioned div (the atomic
   theme styles it `borderLeftColor: accent-bright / 2px`), so it is immune to the
   native-caret compositing glitch. This is exactly what Obsidian does. Verified
   on the Simulator (2026-06-11, v166): the drawn caret stays visible across
   arrow-up/down line moves (a single screenshot can still catch the CSS *blink*
   off-phase — burst-shoot to confirm). **Do NOT "fix" the caret by removing the
   compositing hack** (it exists so typed text repaints inside `position:fixed`).

3. **iOS long-press "Paste / Select" menu — was suspected drawSelection-broken,
   but that was a MISDIAGNOSIS.** The real cause of a missing web-editor edit menu
   on iOS is almost always `-webkit-user-select` / `-webkit-touch-callout` on the
   *editable* text. Audited (2026-06-11): every `user-select: none` in
   `mdlive/styles/inline-preview.css` is on a non-editable decoration widget
   (`.cm-atomic-link-icon`, table/wiki-link [both excluded], `.cm-atomic-search-count`)
   — NONE touch `.cm-content` / `.cm-line`. So the editable text keeps a native
   selection, and `drawSelection()` only draws an OVERLAY (it does not remove the
   DOM selection the iOS edit menu attaches to). Obsidian confirms drawSelection +
   working paste coexist. **Caveat:** AXe synthetic `touch --down --up` does NOT
   trigger the UIKit long-press menu (it's a gesture recognizer, not a DOM event),
   so this is the ONE item the Simulator can't prove — confirm on a real device.
   If a real device ever shows no menu, the fix is a toolbar Paste button (custom
   affordance), **NOT** dropping drawSelection (which reintroduces pitfall #2).
   Status: **expected-fine (no CSS blocker); confirm long-press paste on device.**

4. **A tiny "dot" after the caret / near rendered markdown on iOS.** CM6 wraps
   every widget (the placeholder, each hidden-marker widget) in a srcless
   `<img class="cm-widgetBuffer">`; iOS WebKit renders a srcless `<img>` as a
   broken-image dot. **Fix:** `.cm-widgetBuffer { visibility: hidden }` (keeps the
   0-width layout box → caret positioning unaffected). Status: **fixed (v161).**

5. **Deleting one marker of an empty pair orphans the other.** `closeBracketsKeymap`'s
   `deleteBracketPair` sat below cowboy's `deleteCharBackward`. **Fix:** wrap it
   `Prec.high`. Status: **fixed (v165).**

6. **Fullscreen ↔ inline text desync ("展开/收缩 state 不同步").** The inline
   editor only renders while `!composeFs`, so opening/closing fullscreen REMOUNTS
   it and it re-seeds from its uncontrolled mount seed (`initialDraftText`),
   reverting. **Fix:** on collapse, `initialDraftText.current = text` before
   `setComposeFs(false)`. Do NOT feed `text` as the inline `value` (re-applies
   every keystroke → iOS caret bounce). Status: **fixed (v162).**

7. **Toolbar marker inserts leave the caret BEFORE the marker.** `toggleLinePrefix`
   / `cycleHeading` set no `selection`, so CM maps the caret to line start.
   **Fix:** explicitly move the caret by ±marker length. Status: **fixed (v163).**

8. **`buildDenoViteApp` deps-FOD DNS fails when adding CM6 deps.** Build with
   `nix build .#cowboy-web --option sandbox false` to capture the new `depsHash`.
   (See the columbus memory `deno-vite-fod-dns-sandbox`.)

## Verification matrix (run the WHOLE thing after any editor change)

On the **iOS Simulator** (or device), in BOTH the inline composer and the
fullscreen editor:

- [ ] Type pinyin on a `**`/`# ` marker line — no dropped/garbled chars (pitfall #1).
- [ ] Caret is visible + stays visible while typing/scrolling (pitfall #2).
- [ ] Long-press → the Paste/Select menu appears; Paste works (pitfall #3).
- [ ] No stray dot near the caret / rendered markdown (pitfall #4).
- [ ] `**`/`(`/`` ` ``/`[` then Backspace clears the whole pair (pitfall #5).
- [ ] Type inline → expand → type → collapse: text is preserved (pitfall #6).
- [ ] Toolbar quote/list/heading: caret lands AFTER the marker (pitfall #7).
- [ ] Attach a photo, then type — keyboard returns, input works.
- [ ] Headings/bold/italic/code/list render (inactive line) + reveal (active line).

Desktop (Chrome bridge) covers render + vim + desktop IME, but **cannot** prove
#1–#3 — those are iOS-only.
