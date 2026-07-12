# mdlive — iOS / CM6 pitfalls & the Obsidian-alignment contract

> **RESOLVED ARCHITECTURE CHANGE — task `cowboy-composer-drop-pwa-hacks`.** The
> whole PWA iOS hack tower documented below is **REMOVED IN CURRENT CODE**: the
> composer now runs one native-shell-oriented code path in normal flow.
> Specifically gone — `.cm-scroller { translateZ(0) }`
> (cmTheme), the `compositionstart/update/end/blur` transform+opacity dance
> (ComposerEditor), `drawSelection()` + `dropCursor()` + the `.cm-composing`
> dance (composerExtensions), the `position: fixed` body (index.html), the
> `FullscreenComposer` fixed overlay, and the two unconditional `translateZ(0)`
> repaint nudges (Composer.tsx). Root cause confirmed against the Capacitor
> keyboard source (Obsidian's mechanism): the entire tower existed ONLY to work
> around the PWA's `position: fixed` keyboard lock; the native Tauri shell
> resizes the WebView instead, so none of it is needed.
>
> The resolved entries remain below as WHY-history and a **do not re-add** rule;
> they are labelled historical rather than described as active inventory. Any
> later editor change still requires the verification matrix at the end.

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
| `drawSelection()` | ❌ removed | Native caret/selection is required for the iOS Paste/Select callout. The PWA `translateZ(0)` layer that once required a drawn caret is gone. See historical pitfalls #2–#3. |
| `dropCursor()` | ❌ removed | It was coupled to the drawn-selection workaround and is unnecessary in the native-shell path. |
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
   (d) the current native-shell path has no PWA `translateZ(0)` layer or
   composition transform dance. **Status: verified with Simplified Chinese
   Pinyin in compact + fullscreen Simulator paths, plus the earlier native-shell
   device sign-off. Re-verify on EVERY editor change.**

2. **Historical — caret "经常消失" (blinks out / invisible), esp. when moving
   up/down.** iOS
   WebKit hides the *native* caret when the editor's scroller is a promoted
   compositing layer; vertical caret motion was the worst case. The old fix was
   `drawSelection()`. **Current root fix:** the native shell resizes its WebView,
   so the fixed-body/`translateZ(0)` repaint layer and drawn caret are both gone.
   Do not re-add either half of that retired workaround.

   **2a — historical drawSelection ↔ iOS IME: the drawn caret froze during pinyin
   composition (v166 regression, fixed v167).** drawSelection's `hideNativeSelection`
   facet forces `.cm-content`/`.cm-line { caret-color: transparent !important }` and
   draws its own `.cm-cursor`. CM6 deliberately does NOT re-measure mid-composition
   (so it won't disrupt the IME), so the drawn caret FREEZES at the pre-composition
   position while the marked text ("ni…") renders after it — caret ends up to the
   LEFT of what you're typing ("光标位置很奇怪/UI太丑"). **Fix (composerExtensions.ts):
   composition-aware caret** — a `.cm-composing` class (toggled on
   compositionstart/end + blur self-heal) whose CSS reveals the NATIVE caret
   (`caret-color: accent`, it tracks marked text in the same layer) and hides the
   stale drawn `.cm-cursorLayer` for the duration of the composition; restored on
   commit. That `.cm-composing` workaround was removed together with
   `drawSelection()`; the current path uses the native caret throughout.

3. **iOS long-press "Paste / Select" menu — `drawSelection()` DOES break it (on a
   real device), and the fix is to go native in the shell.** The CSS-blocker
   theory (`-webkit-user-select`/`-webkit-touch-callout` on the editable) was
   audited clean, but a real device (2026-06-11) showed the no-selection menu
   (`Paste | Select | Select All | AutoFill`) flicker up then immediately dismiss
   — because `drawSelection()`'s `hideNativeSelection` forces
   `caret-color: transparent`, so the menu had **no native caret to anchor to**.
   AXe never caught it (synthetic touch isn't a UIKit gesture), so the earlier
   "expected-fine" was wrong. **Root fix:** drop `drawSelection()` +
   `dropCursor()` + the `.cm-composing` dance and use the native caret/selection
   on the single current path. Also: `.cm-content { min-height: 100% }` (cmTheme)
   so the whole editor area is editable — long-pressing the blank space below a
   short note now hits the contenteditable instead of the inert scroller ("长按
   区域小"). Status: **fixed in the native shell (v190); the menu is the OS's, not
   custom — nothing to "migrate". Re-verify the IME matrix since the shell caret
   is now native.**

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

9. **Controlled `value={text}` corrupts IME + bounces the caret ("状态错乱") —
   the composer editors MUST be uncontrolled (fixed v168).** `@uiw/react-codemirror`
   reconciles the doc whenever the `value` prop changes. Feeding the LIVE state
   (`value={text}`, where `onChange` also sets `text`) means every keystroke — and
   every IME composition update — re-applies the value into the editor: it dispatches
   a doc change mid-composition (corrupting pinyin) and bounces the iOS caret. The
   inline composer always knew this (`value={initialDraftText.current}`, a frozen
   seed ref); the FullscreenComposer regressed by passing `value={text}` and was the
   recurring "状态错乱". **Rule: BOTH composer surfaces are uncontrolled** — seed the
   editor from a one-shot ref captured at mount (`useRef(value).current` /
   `initialDraftText`), let text flow OUT via `onChange` only, and empty the doc with
   the imperative `clear()` handle (never via `value=""`). They remount on every
   open/collapse, so the frozen seed is always the current text; the parent syncs
   `initialDraftText.current = text` on collapse to carry edits back. NEVER pass live
   state as `value` to either editor.

10. **Historical — IME marked text was "swallowed" during composition because
    two compositing-layer fixes fought each other (fixed v169, later removed at
    the root).** Two coupled iOS hacks
    in `ComposerEditor.tsx` / `cmTheme.ts`: (a) `.cm-scroller { transform: translateZ(0) }`
    is a compositing layer that forces WebKit to repaint the editable on every input
    (the `position: fixed` body otherwise leaves typed text invisible until a later
    edit); (b) `compositionstart` sets `transform: none` to stop iOS mis-placing the
    pinyin marked-text overlay relative to that promoted layer. The conflict: with the
    layer OFF during composition, the repaint bug from (a) returns — the marked text
    paints to nowhere and looks SWALLOWED. It manifests "只有前面有图片时" because an
    attachment chip's reflow leaves the editor's paint rect stale; the tell is that
    typing a few **direct** keys (spaces) "unblocks" it — a direct keystroke forces a
    repaint, IME composition does not. **Fix:** on `compositionupdate`, nudge the
    **scroller's** opacity for one frame to force the repaint (so marked text stays
    visible while `transform: none` keeps it positioned). Nudge the SCROLLER, never the
    contentDOM — contentDOM is the contenteditable host and mutating its style mid-
    composition risks the IME aborting. The current native-shell path removed
    both the transform and opacity dances; this paragraph is history, not an
    instruction to restore them.

11. **Fullscreen toolbar claimed to be selection-aware but never changed mode.**
    `ComposerEditor` emitted `onSelectionChange`, but `FullscreenComposer` did
    not subscribe. Wire it into local state and show only bold/italic/code/link
    for a non-empty range; empty selection keeps the configured toolbar. This
    changes toolbar chrome only and never rebuilds CM6 state or composition.

12. **A phone soft keyboard can't delete an inline image / @-token / empty pair
    ("输入框无法用删除键删除图片").** The custom Backspace handlers
    (`deleteImageTokenBackward` — the two-stage image delete — plus the empty-pair,
    code-fence, and @-token deleters) are CM6 **keymap `{key:"Backspace"}`**
    bindings, i.e. keydown-only. iOS/Android soft keyboards emit **no Backspace
    `keydown`** — they fire `beforeinput` with `inputType:"deleteContentBackward"`,
    which a keymap never sees. So on a phone those handlers never run; for a block
    image CM6's native atomic-range delete then no-ops (the `inlineImageTrailingLine`
    filter re-adds the line it removed) → the picture is undeletable from the
    keyboard. **Fix (`ComposerEditor.tsx`):** hoist the four deleters into one
    `backspaceChain(view)` and run it from BOTH channels — the existing `Prec.high`
    keymap (physical keyboard) AND a new `Prec.high EditorView.domEventHandlers({
    beforeinput })` that catches `deleteContentBackward` and `preventDefault`s ONLY
    when a handler actually consumed the delete (normal char-deletion falls
    through). No double-delete on desktop: the keymap already `preventDefault`s the
    keydown, so no `beforeinput` follows. Same soft-keyboard gap would hit any
    future keymap-only editing command. **Status: fix verified headlessly through
    the beforeinput channel** (Playwright: a synthetic `deleteContentBackward`
    removes the image after the fix, no-ops before it; keydown path unchanged).
    **NOT yet device-verified** — the full iOS caret/IME/paste-menu matrix can't be
    reproduced in emulated Chrome; confirm on the Simulator/device before landing.

13. **Desktop Vim Normal mode must not toggle `contenteditable` to suppress the
    system IME.** `@replit/codemirror-vim` reacts to composition in Normal mode by
    force-ending it, including temporarily detaching the editable surface. An
    attempted Cowboy guard that dynamically changed `EditorView.editable` made the
    real macOS Chinese IME lose its caret and input context entirely. **Rule:** keep
    the same `.cm-content` DOM node editable for the editor's whole lifetime. The
    Desktop-only `desktop/vim/imeAutoInsertVim.ts` extension listens for a real
    `compositionstart` at higher precedence and moves Vim to a clean Insert state
    before the upstream Vim handler runs. That alone is too late for a physical
    Normal-mode key: macOS can already show a candidate window before JavaScript
    receives a cancellable keydown. **The reliable web boundary is focus, not event
    cancellation.** While Vim is outside Insert, focus lives on a visually hidden,
    non-editable command sink inside the editor. It cannot host native composition;
    physical keys are mapped by `KeyboardEvent.code` and sent directly to Vim. An
    insert command focuses the unchanged `.cm-content`; Escape returns focus to the
    sink. Clicking the editor in Normal also redirects focus to the sink. This makes
    `i`, motions, operators, counts, and punctuation work with a CJK input source
    active without a candidate window. Composition-triggered auto-Insert remains a
    fallback if editable focus is reached by an unusual input path. This module is
    dynamically imported only by the Desktop composer; Mobile must neither load it
    nor acquire Desktop Vim/IME behavior.

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
- [ ] Attach a photo, put the caret below it, press the SOFT-keyboard Backspace
      twice — the image is ringed then deleted (pitfall #11; keydown-only handlers
      don't fire on a phone, so this is the beforeinput path).
- [ ] Headings/bold/italic/code/list render (inactive line) + reveal (active line).

Desktop (Chrome bridge) covers render + vim + desktop IME, but **cannot** prove
#1–#3 — those are iOS-only.

Desktop Vim + IME checks:

- [ ] In Normal and Visual/operator-pending mode, `compositionstart` enters Insert.
- [ ] With a CJK input source active, physical `i` enters Insert without opening a
      candidate window; motions/operators remain Vim commands in Normal mode.
- [ ] `.cm-content` stays the identical DOM node and remains `contenteditable=true`.
- [ ] Composition text is accepted with a visible, focused caret after the switch.
- [ ] Escape returns focus to the command sink and the status line shows `IME SAFE`.
- [ ] At a mobile viewport, the Desktop Vim/IME chunk is not requested and Mobile
      editor behavior is unchanged.
