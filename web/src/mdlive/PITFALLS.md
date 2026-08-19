# mdlive — iOS / CM6 pitfalls & the Obsidian-alignment contract

> **OPEN TODO — pitfall #69 (end of this file).** Physical iPhone Return
> after a pasted composer image still fails: CM6 / `.cm-activeLine` move,
> the painted UIKit caret does not. 2026-08-15 user: same shape in
> Obsidian; likely WeType (first Return a bit wrong on native Pinyin,
> first few Returns wrong on WeChat Input Method). Do **not** claim this
> fixed. Do **not** re-ship Obsidian token reveal. Do **not** start
> another beforeinput / U+200B / Selection-remap patch. Canonical ledger
> (symptom, diagnosis, v1240–v1269 failed attempts, do-not-retry): **#69**
> below. Debug mode is pitfall #68. Simulator CSS caret is not evidence
> (pitfall #67). WeType cannot be installed on the iOS Simulator.

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
| `search({top:true})` + `searchKeymap` | ❌ removed | Stock CM6 Find is Chrome-looking chrome (`Find` / `replace all`). Desktop already refuses to bind `Mod+F` as a Cowboy command; the vendored keymap was sneaking the panel back in. Search the workspace through the command palette. |
| `EditorView.theme({".cm-widgetBuffer":{visibility:"hidden"}})` | ✅ | hide the iOS broken-image dot (pitfall #4). |
| `indentWithTab` | ✅ | Tab indent. |
| `rectangularSelection()` / `allowMultipleSelections` | ❌ | multi-cursor; desktop-only nicety, needs drawSelection to render. Re-add desktop-only if ever wanted. |
| `history()` / `historyKeymap` / `defaultKeymap` | ❌ | **cowboy's `ComposerEditor` base already provides these.** A 2nd `history()` SPLITS UNDO. Never add here. |
| `table-widget` / `image-blocks` / `wiki-links` | ❌ | the only **contenteditable** widgets = the IME landmine; and cowboy has no notes vault for `[[wiki-links]]`. Out of v1. See SYNC.md. |
| `highlightSpecialChars()` | ❌ | renders dots for special chars — noise in a chat box. |
| `initialRevealField` | ❌ | React-wrapper-only (reveal-on-open); no cowboy use case. |

## Known pitfalls (symptom → cause → fix → status)

0. **Desktop Vim block cursor disappears on the text line immediately before a
   block image.** The logical CM6 selection is correct; the failure is in
   `@replit/codemirror-vim`'s cursor measurement. At EOL it walks into the next
   block widget to read font style, then descends through the widget's `<img>`;
   an image has no child node, so that measurement produces no cursor piece.
   **Fix (`inlineImages.ts`):** the image widget owns a zero-size leading text
   node inside a zero-line-height wrapper. It gives the upstream DOM walk a safe
   terminal node without changing document positions, thumbnail geometry,
   atomic ranges, pointer behavior, or Mobile's native caret. Keep block images;
   reverting them to inline makes the caret as tall as the thumbnail.

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

   **Touch pointer and focus ownership (2026-07-26):** the inline-preview
   `freezeMousePlugin` used to freeze decorations for every primary pointer,
   including iOS touch. Its `setFrozen(true)` on pointerdown and delayed
   `setFrozen(false)` 100ms after pointerup changed CM state while UIKit was
   promoting the long-press magnifier into its edit menu, cancelling the popup.
   The freeze exists only to keep Markdown markers from shifting beneath a
   desktop mouse click; it must accept `pointerType === "mouse"` only. Main,
   queued, and draft fullscreen entry points must also transfer focus exactly
   once from their opening user gesture. Delayed 60/120/200/320/400ms `focusEnd`
   retries replace the already-armed native text interaction and suppress the
   same menu. Never restore touch freeze or post-open focus retries.

   **Full-cover Composer settings and `:focus-within` (2026-08-03):** a toolbar
   button becomes `document.activeElement` before its React `onClick` runs. The
   Tune button is still inside `[data-mobile-focus-composer]`, so blurring only
   textarea/contenteditable focus owners leaves the whole Composer matched by
   `:focus-within`: iOS hides the keyboard for the sheet, but the background
   card remains a tall empty focused canvas after the sheet closes. A full-cover
   transition must blur whichever active descendant owns the Composer focus
   region, including its button. Inline formatting and attachment actions still
   preserve editor focus and must not use this helper.

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
    Deleting the final token also changes the hybrid editor owner from CM6 back
    to the native textarea. A plain React replacement leaves `BODY` focused and
    collapses the keyboard. `PlatformComposerEditor` must snapshot the post-delete
    logical selection and exact CM6 focus owner before mirroring the new text,
    then give the replacement textarea one commit-time autofocus plus a one-shot
    selection. Clear that claim only after the native child commits; never retain
    token-free CM6 or repair the handoff with a timer/rAF refocus. **Status:
    Simulator-verified through the real software-keyboard deletion path.**

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
    nor acquire Desktop Vim/IME behavior. Insert commands that move/edit before the
    mode transition (`a/A/o/O/s/S/C/R` and change operators) must focus CodeMirror
    before dispatch. After the transition, resubmit the CM selection on the next
    animation frame, then restore the native Selection from CM6's `requestMeasure`
    queue. If `view.domAtPos(head)` still returns `contentDOM`, retry across at most
    two animation frames; in particular `o/O` create an empty line whose DOM can
    land after the first frame, otherwise leaving the browser selection on the
    content root instead of the new `<div class="cm-line"><br></div>` and making
    the caret intermittently invisible. Apply this stabilization to every direct
    Insert command, not an `o/O` special case.
    The sink is keyboard focus within the editor, so it adds
    `.cm-vim-command-focused`: Normal uses the solid theme-accent block. The block
    cursor is paint, never layout: the empty-document placeholder and the first
    real document character must use CodeMirror's same zero-offset inline origin.
    Do not add a placeholder-only character-cell margin; it makes
    `Message the agent…` jump left as soon as the first character is entered. Do
    not show the upstream pink/hollow "unfocused" cursor while the command sink is
    active.

    **IME focus/Selection ownership (2026-07-13):** `compositionstart` may switch
    Vim into Insert, but it must not call `focus()`, dispatch a selection, or
    collapse `window.getSelection()` until the native composition ends. Doing so
    leaves macOS marked text visibly underlined while its input channel is dead.
    Every deferred caret-stabilization callback must re-check both the local
    composition guard and `EditorView.composing`. Likewise, the Normal command
    sink may only replace focus while its own EditorView still owns focus; a
    mount-time or stale microtask must never acquire focus from another region.
    Desktop surfaces may expose this lifecycle through the Desktop-only IME
    status store: composition is visible only while active, an auto-protected
    Normal→Insert transition is labelled explicitly, and commit confirmation
    expires after 600ms. Do not infer the user's language or input-source name;
    browsers expose composition lifecycle, not the selected OS keyboard layout.
    Direct Insert commands must not all share deferred Selection stabilization:
    a fast `i` followed immediately by pinyin can start native marked text after
    the command's RAF was queued but before `compositionstart` reaches JS. That
    stale RAF then rewrites Selection and leaves underlined romanization frozen.
    Plain `i/I/a/A/R` only focus the content once; reserve deferred DOM/caret
    stabilization for structural commands (`o/O/s/S/C` and change operators)
    that can create an empty line, with composition guards at every phase. A
    composition guard by itself is still racy: on macOS the first physical IME
    key can reach the contenteditable before `compositionstart`. Structural
    repair therefore captures a native-input epoch; any later content keydown,
    beforeinput, or compositionstart invalidates every pending RAF/measure/write.
    Never let a Vim-command callback call focus/dispatch/Selection.collapse after
    native input has begun, even when `EditorView.composing` is not true yet.
    The same command-sink focus means browsers do not paint native selection in
    Visual mode. Desktop therefore owns a small `cm-vim-visual-selection`
    decoration driven by Vim mode plus CM selection. Do not replace it with
    `drawSelection()`: that facet hides the native Insert caret and revives the
    frozen IME marked-text failure described above.

    Insert mode must likewise have exactly one cursor owner. The native caret is
    authoritative while `.cm-content` owns focus; `@replit/codemirror-vim` may
    keep its previous `.cm-vimCursorLayer` alive for one update after the mode
    transition, which paints a second adjacent caret on an empty line. Browser
    focus is not a sufficient mode signal: native WebView can paint the caret
    before `.cm-editor.cm-focused` settles, especially on an auto-continued
    Markdown list where the logical and decorated DOM positions differ. The
    Desktop Vim runtime must synchronously toggle `.cm-vim-native-caret` from
    its actual `insertMode`; the theme hides only the stale Vim cursor layer
    under that class (with `.cm-focused` as a fallback). Command-sink
    Normal/Visual states remove the class and retain their block cursor and
    custom Visual decorations. Never solve this by hiding the native caret or
    adding `drawSelection()`—both break IME ownership. Status: fixed in
    cowboy-v1401 with a cursor-owner contract test.

    Focus calls are also ownership changes. A direct Insert command may focus
    CodeMirror before Vim applies its logical selection, but the following Vim
    mode-change callback and post-command synchronization must detect that
    `contentDOM` already owns focus and become no-ops. Re-focusing the same
    editable two or three times is enough for the first macOS pinyin key to land
    between focus/Selection writes, leaving underlined marked text visible with
    a dead input channel. The Desktop global command layer must additionally
    bypass every key while either the event or the shared IME lifecycle reports
    composition (including legacy keyCode 229), and discard armed focus chords.

    An empty document needs one synchronous exception to the "plain `i` has no
    deferred stabilization" rule. CM6 renders an empty `.cm-line` with placeholder
    widgets plus a `<br>`, but focusing `contentDOM` can leave the browser Selection
    on the content root. The editor is then correctly in Insert mode with native
    caret color enabled, yet there is no paintable caret anchor. During the same
    command-sink keydown, after Vim enters Insert, collapse Selection to offset 0
    of that existing empty `.cm-line`. This is synchronous and precedes any later
    native/IME key; do not turn it into a RAF or apply it to non-empty documents.

    macOS can emit a transient `blur`/`focusout` while its candidate window still
    owns marked text, before `compositionend`. Never clear the Desktop runtime's
    composing flag or normalize Vim on that blur: sending Escape in this window
    leaves an underlined pinyin fragment visible with a dead input channel. Record
    the true focus exit, wait for `compositionend`, then normalize only if focus is
    still outside the editor. Main compose, queued-message edit, draft edit, and
    their expanded editor must all enter through `PlatformComposerEditor`; that
    gateway enables this behavior only for Desktop and forces Vim off on Mobile.

    **Initial extension ownership (2026-07-17):** native composition must never
    span a CodeMirror extension reconfiguration. `@uiw/react-codemirror` applies
    a changed `extensions` prop with `StateEffect.reconfigure`; lazily adding the
    Desktop Vim runtime after the editor is already interactive can therefore
    strand macOS marked text even when every Vim callback has a composition
    guard. `PlatformComposerEditor` must preload the Desktop-only Vim chunk and
    keep its temporary editor disabled, then mount a separate interactive editor
    whose initial `EditorState` already contains Vim. Mobile/touch must not load
    that chunk. If loading fails, prefer an interactive non-Vim composer over a
    permanently disabled input.

    The preload effect must also remain attached to its promise. Do not set an
    intermediate `loading` state from an effect that depends on that same state:
    the resulting render cleans up the effect, invalidates its completion
    callback, and leaves the temporary editor permanently `contenteditable=false`.
    A single `pending` state owns both the in-flight promise and the disabled
    mount; only promise completion may transition it to `ready` or `failed`.
    The Desktop workspace may designate `prompt.composer` before its lazy subtree
    exists. `DesktopWorkspaceProvider` owns reconciling that state: observe only
    until the selected region mounts, apply its focus markers, and focus it only
    while `BODY` still owns focus. Disconnect immediately afterward; never make
    an editor-local callback guess workspace state or steal a newer user focus.

14. **A narrow Desktop window is not the Mobile product.** Do not derive
    composer mode from a viewport breakpoint. `surface === "touch"` alone owns
    Mobile's overlay composer, bottom navigation, detent sheets, and progressive
    disclosure. Desktop defaults to the split Prompt + Conversation workspace;
    below 1100px only its Sessions rail may move into a drawer, while the Desktop
    composer stays in column mode with its queue/drafts in a bounded internal
    scroller. This layout rule changes no CM6 extension or editor DOM and keeps
    the Desktop-only Vim/IME chunk isolated from Mobile. Regressing to
    `mobile = touch || narrow` makes queued/draft panels consume the viewport and
    strands the production editor at the bottom—the exact failure the separate
    product shells are intended to prevent.

15. **Deleting an inline image token must also delete its attachment block.**
    The literal CM6 document and React's `attachments[]` are separate sources:
    Backspace/Delete can remove `![name](cowboy-att:id)` while leaving the image
    bytes in the array. Saving a queued message or draft then sends that stale
    block and the server restores the image, so it appears to survive deletion.
    Every editor text-change path (main compose plus queued/draft edit, inline or
    fullscreen) must run `reconcileDeletedInlineImages(previous, next, attachments)`.
    It removes only image attachments whose token existed in `previous` and no
    longer exists in `next`; non-image files and legacy gallery-only images must
    remain. Keep a synchronous previous-text ref because CM6 changes can arrive
    faster than React renders.

16. **Desktop Vim state must not leak across workspace focus changes.** A
    Composer, queued-message editor, or draft editor can remain mounted after
    focus moves to Sessions, Conversation, Top Bar, or another Prompt region.
    Upstream Vim therefore kept Insert/Visual/operator-pending state alive and
    rendered macro recording as an unthemed inline `recording @x` dialog.
    **Fix (`desktop/vim/imeAutoInsertVim.ts`):** a true focus exit from the
    editor (not the internal contenteditable ↔ command-sink handoff) sends one
    Vim Escape, clears partial commands, and stops any active macro. Intercept
    only upstream's macro-recording dialog and publish it to the Desktop status
    line as `REC @x · Q Stop`; all other Vim search/command dialogs retain their
    upstream behavior. This runtime remains dynamically imported by Desktop
    only; Mobile receives neither the focus reset nor the macro UI. Prompt
    region shortcuts use modified global chords (`Mod+P/I/Y/D`) so bare Vim
    paste, macro, delete, and motion commands always remain editor-owned.
    The same CJK input source can mark any physical Normal-mode keydown on the
    non-editable sink as `isComposing`/229 even though no marked-text transaction
    exists there. Do not discard those events: route the complete letter,
    punctuation, motion, operator, count, and special-key map through
    `event.code` and `vimCommandKey`. Only the runtime's actual
    `compositionstart`/`compositionend` lifecycle (and `EditorView.composing`)
    may suspend sink commands. This exception ends at the Normal sink. Everywhere
    else, `isComposing`, keyCode 229, and native text-service keys such as
    `Process` remain exclusively IME-owned even when focus has moved onto a
    button, list, or modal shell; workspace/list/config shortcuts must ignore
    them before resolving any physical key.

17. **Queue/Draft disclosure and edit ownership are separate state machines.**
    These panels use the same MUI `Collapse` motion as Plan, and non-editing rows
    stay mounted so attachment previews and sortable registration are not
    recreated by every fold. An active edit is different: it owns Mobile's only
    writing surface and therefore always forces the panel visually open. Never
    let a persisted disclosure preference hide an unresolved editor while the
    parent still suppresses the ordinary composer. Desktop keeps an explicit
    edit transaction: a clean edit may be abandoned and folded immediately; a
    dirty edit must resolve through Keep editing, Discard, or Save and collapse.
    Mobile is touch-first: its row buffer auto-commits when the software keyboard
    is dismissed, then exits editing and restores the ordinary Queue/Draft card.
    Its primary accessory action is therefore Hide keyboard, never a redundant
    Save checkmark. Bulk send, clear, and reorder actions stay disabled until the
    active edit resolves. Composer height is observed by the
    single persistent ResizeObserver in `App.tsx`; do not add a panel-local
    observer or per-frame React state. The Collapse height layer uses
    `will-change: height`, and its wrapper inner uses `contain: layout paint` so a
    large attachment preview is laid out once instead of producing a long paint
    stall during every reveal frame. Re-verify touch scrolling and inline-editor
    focus after changing this animation. Mobile row edits begin in this compact
    card instead of navigating directly to fullscreen. The card uses the shared
    two-track accessory primitive and exposes one explicit expand action; opening
    fullscreen transfers the same draft and attachment state without changing
    transaction semantics.

18. **Queued/Draft edit completion follows the input model.** Desktop local text
    and attachments remain an explicit transaction until Done/Save or Discard.
    On Mobile, dismissing the software keyboard (system gesture or the accessory
    button) auto-commits a non-empty buffer, exits inline/fullscreen editing, and
    restores the default pending card; an emptied buffer reverts rather than
    creating an invalid empty message. The main new-message composer remains
    independent: keyboard dismissal only changes its focus/layout and never
    submits or discards its draft. Keep these semantics explicit instead of
    inferring them from labels.

19. **Slash commands require explicit completion provenance.** ACP transports
    commands and ordinary prompt text through the same string field, so text
    alone cannot distinguish a selected `/skill` from a typed `/dir` or absolute
    path. Both CM6 and the native-textarea picker retain intent only when the
    user clicks or keyboard-confirms a completion. At the user-submit boundary,
    an unselected leading slash receives an invisible U+2060 WORD JOINER; this
    prevents agent command dispatch without changing the visible editor text or
    path. Never infer intent merely from a command-shaped string, never blanket
    escape slashes, and keep app-generated lifecycle commands such as `/compact`
    outside this user-composer guard.

20. **Fullscreen row-edit chrome is one non-selectable two-track keyboard dock.**
    The upper track owns only horizontally scrolling formatting commands. The
    lower track owns completion and editor-level actions, with Settings fixed in
    its final 44pt slot. This keeps the completion action stable without mixing document formatting
    and message lifecycle in one crowded rail. Do not restore the wide Save pill,
    opaque detached rows, or a dismissal pill: on iOS each wastes the
    keyboard-adjacent area and detached labels can acquire native text-selection
    handles. The dock and its controls use `user-select: none`; this applies only
    to chrome, never the CM6 canvas, so native caret, selection, long-press Paste,
    and IME behavior remain unchanged. Desktop row editing retains Done; Mobile
    shows Hide keyboard, auto-commits the row buffer when dismissal completes,
    and returns to the default Queue/Draft card. The top-right
    ignore-modifications action and confirmation dialog remain separate.

21. **The native iOS shell can intermittently leave a gray strip above the
    keyboard when UIKit's predicted keyboard frame is taller than its final
    layout.** `UIKeyboardWillChangeFrame` and even `UIKeyboardDidShow` are not a
    durable geometry source: predictive/IME bars and third-party keyboards can
    settle in later phases without another dependable notification. The web
    must keep `--kb-inset` disabled in the native shell; applying
    `visualViewport` overlap there double-lifts the composer. The native
    `CowboyKeyboardAvoider` instead follows notification frames during the
    animation, then reconciles the WKWebView at 0/120/350/700ms against the
    owning view's authoritative iOS 15+ `keyboardLayoutGuide.layoutFrame`.
    Generation guards cancel stale corrections on a newer frame or hide. Ignore
    guide overlaps below 80px so a hidden keyboard's safe-area guide cannot
    shrink the WebView. Simulator verification must include repeated keyboard
    close/open cycles, equal layout/visual viewport heights, settled-overlap
    logs, and a screenshot with no band between app chrome and keyboard.

22. **An iPad split/floating keyboard can leave the composer and navbar near
    the top with a large blank region below.** UIKit first reports a predictive
    full-width keyboard end frame, so shrinking the WKWebView from that frame
    makes the web layout only a few hundred points tall. The default
    `UIKeyboardLayoutGuide` does not follow undocked keyboards; its collapsed
    safe-area frame therefore could not correct that early shrink. The native
    avoider must set `followsUndockedKeyboard = YES`. Both notification and
    settled-guide geometry resize the viewport only when the keyboard frame
    reaches the parent's bottom edge: a docked split keyboard receives its real
    bottom overlap, while a genuinely floating keyboard overlays the document
    and leaves the WKWebView full-height. A collapsed (<80px) guide remains a
    transient non-authoritative reading. Do not compensate in web CSS or
    `--kb-inset`; that would double-lift ordinary docked keyboards. Verify on an
    iPad-sized Simulator with repeated docked keyboard open/close cycles and on
    a physical iPad with split and floating layouts.

23. **Fullscreen Mobile needs an explicit way to inspect behind the keyboard.**
    Keep one trailing dock slot for keyboard ownership. While the software
    keyboard is up it is `KeyboardHide`: blur the first-responder only. It must
    not reconfigure CM6, mutate the document, collapse the fullscreen surface,
    auto-close on a later `visualViewport` close, or add a detached dismissal
    pill. Collapse stays on the upper-track CloseFullscreen control (and Escape).
    After the keyboard is already down, the same slot is Edit: `focusEnd()` in
    the same tap so iOS raises the keyboard with the caret at the document end.
    Tapping the editor canvas still restores native focus normally.

24. **Mobile session-title editing is explicitly saved from the sheet footer.**
    While the title field is focused or contains an unsaved change, replace the
    centred liquid-glass Close glyph with a check-shaped Save action. A plain
    soft-keyboard Enter must never rename the session; only tapping Save commits.
    Keep the pending action mounted through pointer-down so WebKit cannot blur
    the input, swap the footer back to Close, and swallow the tap. Closing the
    sheet discards the local title draft. This transaction is separate from the
    CM6 composer and must not add editor keymaps or IME interception.

25. **The first iPad keyboard presentation after rotating needs current-orientation
    native geometry.** A keyboard notification can briefly combine the new
    full-width keyboard with an old-orientation bottom coordinate. Treating that
    frame as undocked leaves the WKWebView full-height, so the docked keyboard
    covers the composer until a second, warm presentation. On a real orientation
    change, `CowboyKeyboardAvoider` must invalidate the old generation, restore
    the WebView to the parent's current bounds, and then settle from
    `keyboardLayoutGuide`. A substantial full-width notification is a docked
    fallback even if its transient bottom coordinate misses the edge; genuinely
    floating iPad keyboards remain narrow and continue to overlay. Reconcile the
    guide for a bounded two-second window because iPad interface coordinates may
    settle after the old 700ms final sample. **Both the full-width fallback and a
    bottom-edge intersection must be bounded by the keyboard rectangle's
    orientation-invariant short edge.** `convertRect` can swap a stale frame's
    axes and make its old screen width look like the current vertical depth;
    trusting either the converted `height` or an unbounded
    `parentBottom - frame.minY` then shrinks a 1376pt WebView to about 480pt and
    recreates the large blank region above an iPad split keyboard. An ordinary
    current-orientation frame still resolves to the same bottom intersection.
    Ignore face-up/face-down device motion so ordinary handling cannot reset a
    visible keyboard. Verify the exact cold sequence: keyboard hidden → rotate →
    first focus, in both directions, plus split and floating keyboard regressions.

26. **Mobile compact and fullscreen composers must preserve the same inline-image
    tokens.** Replacing compact CM6 with a native textarea made long-press text
    interaction reliable, but the follow-up workaround stripped
    `![name](cowboy-att:id)` tokens and rendered every image in an external tray.
    That destroyed document position, so expanding could not restore the image
    inline. `PlatformComposerEditor` therefore uses a deliberate hybrid: compact
    touch text with no image token mounts the native textarea so UIKit owns the
    caret and long-press menu; inserting the first image writes its placement
    token at the native caret and promotes the unchanged document to CM6 so the
    widget renders inline. Fullscreen and fill layouts follow the same hybrid:
    native while token-free, CM6 while an inline token needs its widget. Keep Vim
    disabled on touch and treat the token as the placement authority across
    compact → fullscreen → compact remounts. The attachment tray is only for
    ordinary files and legacy images without a token. Never delete placement
    tokens or move token-backed images out of the editor to repair interaction.
    Drafts already damaged by the retired demotion path cannot recover their
    original coordinates; on restore, append each unplaced image in attachment
    order and mint a token so it rejoins the authoritative inline lifecycle.

27. **Pasting the first image in compact Mobile must atomically transfer both the
    document and native keyboard focus.** Compact starts as a native textarea,
    but an inline-image token promotes it to CM6. Inserting asynchronously
    converted files one-by-one reused the textarea's stale render-time value, so
    later files could overwrite earlier tokens; unmounting that focused textarea
    after async encoding also dismissed the keyboard. A deferred `focus()` cannot
    reclaim UIKit's software-keyboard session once the Paste gesture has ended.
    Clipboard extraction must inspect both
    `DataTransfer.files` and item-only file entries used by iOS. Register all
    an object-URL placeholder and insert every token synchronously from the live
    DOM value/selection during the Paste event. React then freezes that token-bearing
    document as CM6's one-shot promotion seed and autofocuses the replacement in
    the same discrete UIKit gesture. Encode the durable ACP bytes asynchronously,
    disable send/persistence while placeholders are pending, replace them by stable
    id, refresh the widgets, and revoke their object URLs. File-picker attachment remains intentionally
    non-refocusing because picker dismissal owns a separate UIKit lifecycle.
    Never repair this by externalizing inline images or feeding live React text
    back into CM6.

28. **Desktop transactional-edit Escape must follow Vim mode ownership.** A
    queued/draft row once captured every Escape on its outer Paper before
    CodeMirror could process it. The first Insert-mode Escape therefore opened
    the discard dialog instead of merely returning to Normal. The outer capture
    remains necessary because Normal focus lives on the non-editable command
    sink, but it must query the mounted editor's actual CM Vim state. Insert,
    Visual, operator-pending, and partial key-prefix states belong to Vim; only
    plain Normal may delegate Escape to Cowboy's discard/stop chrome. Do not use
    asynchronously mirrored React mode text for this boundary, and do not move
    the check to Mobile or alter the contenteditable node. Verify first Escape
    Insert to Normal, Visual to Normal, and a second Normal Escape to the modal.

29. **Desktop CM6 callbacks must be stable across native composition.** A live
    production trace showed that the document transaction was followed by two
    effect-only editor dispatches for every typed character. The editor DOM and
    `extensions` identity stayed stable, but `ComposerWorkspace` recreated its
    draft `onChange` callback after mirroring each character into React.
    `@uiw/react-codemirror` includes `onChange` in the dependency list of the
    effect that dispatches `StateEffect.reconfigure`, so the callback churn
    reconfigured CM6 during macOS marked text. The visible failure is an
    underlined pinyin fragment left behind while the normal editor paint appears
    to vanish; the performance failure is a delayed next paint after otherwise
    short input processing. **Fix (`ComposerEditor.tsx` and
    `useComposerDraftController.ts`):** pass one lifetime-stable ref bridge to
    `@uiw`, and keep Desktop's uncontrolled document/draft hot path in refs plus
    the already-debounced draft store. React only observes semantic transitions
    such as empty to sendable or attachment placement; native touch controls keep
    their live React mirror. `reconcileDeletedInlineImages` must preserve the
    attachment-array identity when no image was removed. Never pass a render-time
    callback directly to `@uiw`, and never restore per-character Desktop text
    mirroring merely to enable an action—the authoritative ref already supplies
    submit, park, schedule, and persistence.

30. **Inline-image actions must not own focus.** MUI `Popover` is backed by a
    `Modal` and `FocusTrap`; opening it for Preview/Delete moved focus away from
    CM6, which immediately ended iOS's software-keyboard session. Render this
    contextual toolbar with non-modal `Popper`, keep the editor's native
    selection active, and prevent its buttons' pointer-down default so Delete
    does not focus chrome before it edits the document. Preview may intentionally
    enter the lightbox afterward. Keep action glyph sizes rem-based so the global
    font-size setting scales them with their labels. Never repair this with a
    delayed editor `focus()`: once the original UIKit gesture has ended, that is
    too late to preserve the keyboard session and can regress long-press menus.

31. **The iOS paste-permission alert temporarily hides the focused textarea.**
    Accepting “Cowboy would like to paste” resumes the native `paste` event, but
    `document.activeElement` can be `BODY` while the system alert returns. The
    first inline image still replaces compact Mobile's native textarea with CM6;
    checking only the active element therefore mounted CM6 without autofocus and
    dismissed the keyboard with the old textarea. `PlatformComposerEditor` now
    records an image-paste promotion intent synchronously in the resumed paste
    callback and consumes it in that same native-to-CM6 React commit. Non-image
    paste and file-picker attachment do not set the intent. Do not replace this
    with a timer or post-mount refocus: neither can inherit the original UIKit
    keyboard transaction, and both risk the native long-press menu.

32. **The whole visible compact input must be the native editing hit target.**
    MUI's default multiline `OutlinedInput` puts its vertical padding on a
    non-editable wrapper, so the 44px field contained only a roughly 14px-high
    textarea. A long press in the visually valid top/bottom inset therefore hit
    a DIV; UIKit treated it as an outside touch, and small long-press drift could
    additionally let Cowboy's pager or Session drawer capture the gesture and
    dismiss the keyboard. Move multiline padding onto the textarea, make that
    editable element at least 44px high, and mark both native and CM6 composer
    wrappers as pager/drawer-ignore regions. The writing surface owns every
    touch that starts inside it; navigation gestures begin outside it. Do not
    paper over an inert inset with delayed focus, pointer-down selection writes,
    or a larger non-editable wrapper.

    The same rule applies after an inline image promotes the focused compact
    textarea to CM6: propagate the expanded editor area's height through
    `.cm-theme-none`, `.cm-editor`, and `.cm-scroller` so `.cm-content` truly
    fills the visible canvas. A `min-height: 100%` contenteditable without that
    resolvable ancestor height still collapses to one text line and leaves the
    apparent writing area inert.

33. **Mobile formatting and completion actions share one two-track keyboard
    accessory dock.** The former single rail mixed Markdown transformations with
    keyboard, attachment, and completion lifecycle actions; overflow then made the
    stable Settings area look like a selected column. `MobileComposerAccessoryDock`
    now owns one 96px material with two semantic tracks. The upper track owns
    attachment, message lifecycle, and the non-scrolling right-edge primary
    group. The lower track owns Undo/Redo/Paste plus horizontally scrolling
    formatting; Hide keyboard is its non-scrolling final 48px slot nearest the
    keyboard. Fullscreen editors render that material as
    the same inset, rounded panel used by the compact composer, not as two
    edge-to-edge system bars. Both surfaces use the same 8px Mobile composer
    gutter; do not introduce a separate fullscreen inset. Its only internal
    separator is the quiet horizontal boundary between semantic tracks. Settings
    is an ordinary trailing 44pt formatting action inside the scrollable group,
    never a sticky selected-looking rail, gradient, or separately divided region.
    Main fullscreen compose also moves Collapse into the upper
    message-action track and uses that track's flexible center for Save Draft,
    Schedule, and state-gated Force Push; do not restore a mostly empty top app
    bar. The overlay
    itself must retain `safe-area-inset-top`: removing the app bar also removes
    the inset it used to consume, otherwise the first editor line sits beneath
    the iPhone status bar/Dynamic Island. Draft and
    Queue keep their separate overlay discard action and reuse the dock with Done
    editing, omitting the main composer's delivery actions.
    An inline Queue/Draft editor is the sole Mobile writing focus while active:
    the ordinary new-message composer yields its slot and the shared pending
    scrollport may grow to 56vh. Do not stack two complete composers above the
    keyboard or retain the ordinary 40vh list cap around the active editor.
    While any Mobile Composer has focus, its complete visible card owns every
    touch that begins inside it, including blank editor canvas, attachments, and
    both accessory tracks. The shell pager and Session drawer must test the
    closest `[data-mobile-focus-composer]` for `:focus-within` before reserving a
    horizontal gesture. Protecting only the textarea or CM6 content node leaves
    the deliberately generous writing canvas as an accidental navigation zone.
    Keep this focus-scoped: an unfocused compact card may still participate in
    the normal spatial navigation model.
    Keep every action in an equal 44pt slot:
    Send/Done uses primary color for hierarchy, not a circular container. Keep this dock in the
    WebView and adjacent to the native-resized keyboard, with a 6px outer breathing
    gap rather than padding either action track. That gap is owned by the surface
    positioning the complete composer, never by `MobileComposerAccessoryDock`
    itself: otherwise fullscreen compose double-counts it while an embedded dock
    gets none. In the native shell, a focused fullscreen editor must not add
    `safe-area-inset-bottom` above the already-resized WKWebView; use the 6px gap
    alone until focus leaves. Moving it into a native
    `inputAccessoryView` would split CM6 command/selection authority across a
    bridge. Compact compose reuses this interaction model as one adaptive dock:
    its message-action track is present at rest, and the formatting track expands
    above it while the persistent editor owns focus.
    Settings remains an ordinary equal-width formatting action, never a sticky
    contrasting block. The fixed lower slot is reserved for Hide keyboard across
    main, Queue, Draft, compact, and fullscreen surfaces. Focus may increase the editor canvas, but the card radius
    and horizontal outer edge stay on the shared Mobile panel geometry. Do not
    animate margin or width on focus: that makes the right border jump and reads
    as a different component replacing the compact composer. Coordinate the bottom session-nav
    exit with CSS `:focus-within`; the same focus transition may inset and round
    the whole composer into a keyboard-adjacent input card, but must preserve
    Cowboy's complete action rows rather than substituting another product's
    button model. Never mirror focus through controlled editor
    state. Toolbar changes must not add focus retries, controlled editor state,
    editor remounts, or a second floating compact-composer toolbar.
    Every pointer-operated accessory button prevents its pointer-down default so
    MUI cannot take focus from the editor before activation; click semantics and
    keyboard accessibility remain intact. The fixed primary slot must also
    commit a stationary touch on reliable `pointerup` and suppress the duplicate
    compatibility click: iOS WebKit can otherwise swallow the first Collapse or
    pending-edit completion tap during a focus/keyboard transition. Fullscreen
    Collapse is the one action that replaces the focused editor, so it alone
    owns the non-scrolling right-edge primary slot. Send/Queue remains the final
    message action before the flexible spacer; grouping it with Collapse falsely
    makes delivery look like view chrome and wastes the useful center slot.
    Synchronously mount the
    compact editor and transfer focus within the originating gesture. Never
    replace either rule with a timer, which runs after UIKit has ended the
    keyboard transaction.

34. **Mobile scrollback batches are measured in visible rows, not cursor
    pages.** History cursors are byte bounded, so a tool-heavy page may render
    only a few cards. One upward gesture keeps fetching sequentially until at
    least ten new `[data-key]` transcript rows have mounted and the reserved
    skeleton band is filled, or history ends. Keep the ten-page ceiling as a
    pathological-session safety bound. A remaining `beforeSeq` owns a quiet
    page-head skeleton group. Promote it before I/O and replace it from its lower
    edge as older rows mount. The placeholder
    should resemble compact thought headings plus tool cards; do not regress to
    a delayed, detached, or floating loading overlay. `loadOlder` must remain
    bounded by a client timeout and report cursor progress. HTTP completion is
    not mounted-content completion: iPhone render pacing may defer the new React
    rows, so replacement geometry must wait for a real row-count or content-height
    change before measuring. A suspended fetch or unchanged cursor must collapse
    the active band instead of leaving Mobile behind a permanent expanded skeleton.
    Permit one delayed automatic retry while the boundary remains near the
    viewport; after that, expose an explicit Retry action. Do not leave the
    threshold consumed with an inert loading-shaped block. A request uses compact
    thought-plus-tool rows; mounted content consumes the expansion back to the
    next quiet page-head group. Automatic visible-boundary requests are guarded
    once per `beforeSeq`, not once per session: advancing the cursor must re-arm
    the guard so a newly visible skeleton converges without requiring a scroll.
    Retained projections apply their saved offset after the first layout pass,
    so boundary detection must converge across the bounded viewport-restoration
    window rather than trusting one `requestAnimationFrame` measurement.
    A failure replaces the skeleton with a
    visible Retry row rather than clipping the action above placeholder cards.

35. **Desktop inline expansion is not a cross-surface preference.** The
    persisted `composer-expanded` and drag height describe Desktop's resizable
    inline Prompt canvas only. Mobile and tablet use their explicit fullscreen
    sheet for long-form writing; their ordinary composer must always pass
    `expanded=false` and no persisted height to the platform editor. Browser
    storage can survive a Desktop/iPad surface-classification change, so gating
    only the expand button is insufficient: gate the editor props at the shared
    workspace boundary. Otherwise a stale Desktop `true` silently promotes an
    empty Mobile native textarea to a 48vh CM6 canvas, producing a large blank
    composer while the UI still presents Mobile's fullscreen affordance.

36. **Fullscreen Mobile blank space must be real contenteditable height, not
    `scrollPastEnd()` padding.** `scrollPastEnd()` writes a viewport-sized inline
    bottom padding onto CM6 content. It is useful for reading/scroll positioning,
    but on iOS the visually empty area is an unreliable native edit-menu anchor:
    long press works near a real line yet often does nothing across the expanded
    canvas. The compact image-promoted editor was reliable because its complete
    height chain resolved to `.cm-content`. Fill mode now follows that same model:
    wrapper, theme, editor, scroller, and content all resolve to the visible height,
    while the `scrollPastEnd()` extension is omitted. Never compensate with a
    pointer-down selection dispatch or delayed focus; both cancel UIKit's active
    long-press recognizer.

37. **A modal Composer settings sheet must end the editor focus session before
    it paints.** Accessory actions normally prevent pointer-down default so
    formatting, attachment, and inline controls do not dismiss the iOS keyboard.
    Toolbar Settings is different: it opens a full-cover DetentSheet, which hides
    the keyboard. If the old textarea/CM contenteditable remains
    `document.activeElement`, the compact card's `:focus-within` height survives
    without a keyboard and leaves a large empty Composer after the sheet closes.
    The Settings trigger therefore releases only the active element whose
    closest owner is `[data-mobile-focus-composer]`, synchronously before it
    opens the sheet. This applies to an in-Composer toolbar trigger, the
    navbar's Session settings trigger, and the global App Settings trigger.
    Both navbar triggers sit outside that owner and must release focus on
    pointerdown before WebKit transfers focus to the navbar button.
    `ComposerToolbarSettings` repeats that narrow release in a
    layout effect only as a fallback for non-toolbar callers. Waiting for the
    effect alone is unreliable on iOS because modal focus transfer can replace
    `document.activeElement` before it runs while the old card still paints its
    focused geometry. Do not move that blur to the shared accessory button
    primitive: ordinary formatting and attachment actions must continue to
    preserve keyboard focus.

    The same scoped release marks the successful end of a Mobile delivery action.
    Send/Queue, Jump to front, Save draft, and confirmed Force push keep the
    editor alive until their authoritative operation resolves, then release both
    the editor and any toolbar button that became `document.activeElement` during
    the touch. Schedule releases after its local store commit. Confirmation
    cancellation, rejection, and invalid/no-op actions retain the keyboard and
    draft. Modal/Popover teardown can restore the old editor focus, so the shared
    dismissal repeats after the next paint. Desktop never uses this touch keyboard
    boundary.

38. **A full-height touch Composer without inline images stays a native
    textarea.** Resolving the complete CM6 height chain makes the blank canvas a
    real `contenteditable` hit target, but physical iPhone testing shows that
    WebKit still anchors its edit menu unreliably when a long press is far from
    the nearest real text line. The compact editor did not reproduce the bug
    because UIKit owned a native textarea. Fullscreen and expanded touch editors
    now use that same native control for plain token-free prose. Any *complete*
    construct mdlive already renders — heading, list, quote, fence, HR, setext,
    `*italic*` / `**bold**` / `_em_`, `~~strike~~`, `==highlight==`, `` `code` ``,
    and `[label](url)` / `![alt](url)` — promotes the same document to CM6 so
    Obsidian live preview can hide inactive-line markers. Incomplete pairs
    (`*hi`, `==`) stay native so IME can finish the wrap before the editor
    swaps. An inline-image token still promotes the document synchronously to
    CM6 so its widget remains in flow. Capture the native caret on markup
    promotion and inherit the keyboard in the same commit, the same way image
    paste does. Keep the live native value separate from CM6's frozen mount seed.

    Still not live-previewed (same as the v1 mdlive exclusions): GFM tables,
    `[[wiki]]`, `$math$` / `%%comment%%`, and callouts. Those either need a
    contenteditable widget (IME landmine) or have no lezer/mdlive node yet.
    Do not widen this list without adding a real inactive-line decoration.

    Do not split long-press into a UIKit path near text and a Cowboy-drawn Paste
    fallback in the blank canvas. The two menus have different appearance,
    placement, permissions, and gesture semantics, so an empty document appears
    to use a different editor. The fullscreen textarea already fills the visible
    writing canvas; leave its touch sequence untouched and let UIKit own the edit
    menu everywhere, including when the document is empty. The app may handle an
    actual native `paste` event for attachments, but must not intercept the hold
    that presents it. Do not replace this with a transparent overlay, synthetic
    pointer selection, `contextmenu`, or delayed refocus: those either suppress
    native selection/IME or cannot summon iOS's menu reliably.

    Keep the obsolete blank-canvas clipboard reader deleted as well as its
    visual menu. A dead `navigator.clipboard.read()` fallback makes it too easy
    to accidentally restore a second paste interaction later. Ordinary native
    paste still reaches the textarea's real `paste` event after UIKit completes
    the gesture. The fixed native-shell image action in pitfall #59 is a separate
    explicit button; it must not add a canvas overlay or browser clipboard
    preflight.

39. **Native-to-CM6 image promotion transfers selection, not only text and
    focus.** The first inline-image token immediately unmounts the native iOS
    textarea, so its `selectionStart` no longer exists when CM6 constructs its
    initial state. Record the caret at the end of the inserted token before the
    controlled `onChange`, then pass it as CM6's initial `selection` in the same
    render that transfers the token-bearing document and keyboard focus. A
    later `focusEnd()` is incorrect: it moves past unrelated trailing text and
    is too late to make the native-to-CM6 handoff atomic. Treat the previous
    editor kind and the pending caret as commit-owned state: React may replay or
    supersede render attempts, so mutating/clearing those refs during render can
    make the committed attempt see "already CM6" and fall back to offset 0.
    Release the one-shot caret and paste-focus claim only from a layout effect
    after the CM6 child has committed its initial EditorState.

40. **Starting a Mobile Queue/Draft edit must mount and focus the real editor
    inside the initiating tap.** Updating the parent `editingId` normally leaves
    the editor mount batched until after the click handler returns. A layout
    effect wrapped in `requestAnimationFrame` is later still: the card appears,
    but iOS has already closed its user-activation window and will not raise the
    software keyboard. For touch editing, synchronously commit the row editor
    with `flushSync`, then call `focusEnd()` on that real textarea before the tap
    returns. Do not substitute a hidden keyboard claim; it can preserve a raised
    keyboard across an existing focus transfer, but it does not arm the newly
    mounted editor's native Paste/Select recognizer. Do not repeat focus on the
    next frame, because that second programmatic focus can leave a caret visible
    while WebKit declines the keyboard.

41. **Mobile Composer focus and collapse use one motion timeline.** The editor
    canvas, formatting row, card surface, and bottom Session bar all use
    `mobileComposerFocusMotion`. On collapse, formatting controls fade first
    while their row and the editor height settle together; the Session bar
    rejoins during that same motion. On expansion, the Session bar yields
    immediately and controls appear after the card has started opening. Do not
    tune those durations independently: mismatched height, opacity, and padding
    transitions make keyboard dismissal look like several clipped panels.
    Preserve the same 4px stack gap between the transcript hairline and the
    first Composer surface as between Plan, Pending, and Composer children;
    removing it visually fuses the transcript and input surfaces.
    Once a real touch editor owns the visible keyboard, switch presentation to
    one floating Composer card: keep Plan/Queue/Draft state mounted but hide its
    surrounding panels, and suppress the shell's full-width glass and hairline.
    The focused card must then own its own clipped `backdrop-filter`; a translucent
    tint alone leaves transcript glyphs readable through the editor, especially
    in light mode. Keep the prefixed WebKit property beside the standard one.
    Queue/Draft edits keep their transaction-owning scrollport mounted while
    stripping its header, sibling panels, frame, and every non-editing row. The
    outer pending region must not scroll in this state; only the active editor
    owns any content overflow. Continue measuring the invisible shell so
    transcript bottom clearance follows the focused card.
    The transcript boundary uses one constant 4px separation across idle,
    focused, fullscreen, and Queue/Draft edit transitions; do not key that inset
    on focus, because fullscreen handoff otherwise makes the gap jump. Internal
    stack children use that same 4px rhythm so optional Pending panels have
    equal space above and below. Keep destructive Clear out of the card's
    top-right utility rail; only the compact main editor's Fullscreen action
    remains there. During keyboard focus, keyboard dismissal owns the lower
    bar's fixed trailing action slot and Clear stays in the scrollable
    message-action track above it. The touch editor area
    keeps an 80px minimum so that compact fixed rail never manufactures a tall
    blank canvas or overlaps itself; content growth remains textarea-owned.
    Keep keyboard dismissal outside every horizontal scroller, so iOS
    rubber-band cannot move it. Clear belongs immediately after Schedule in the
    scrollable delivery actions and stays mounted disabled when there is no
    clearable text or attachment.
    Keep Force push earlier in the scrollable action sequence and Schedule next
    to the fixed rail; two lightning-shaped glyphs beside each other make the
    terminal cluster visually ambiguous.
    Keep that dismissal control outside the horizontally scrollable action
    track: iOS rubber-band moves every descendant of the scroller, including a
    `position: sticky` child, before sticky settles back into place.
    The scrollable action track uses compact fixed gaps rather than
    `space-evenly`: distribution space made adjacent Schedule and Clear actions
    look unrelated and hid the fact that the row could scroll. Fade only the
    edge that has offscreen content, measured from the real scroll extent; the
    affordance disappears at either end and never animates either fixed trailing
    group with iOS rubber-band.

42. **The Mobile keyboard-dismiss action is an explicit focus boundary, not an
    editor mutation.** Keep the button in the lower editing track's fixed final
    slot and reveal that track from the composer's native `:focus-within` state; do not mirror
    focus or keyboard visibility through React state. Its `pointerdown` must
    prevent the button from stealing focus, and its click must call the shared
    keyboard-dismiss helper without changing text, selection, attachments, or
    expanded state. This makes the keyboard collapse predictably while leaving
    the draft exactly where the user stopped typing.

53. **Page View's conditional Composer must not unmount on delivery intent.** The
    mobile Page Dock mounts the new-question Composer only while its local
    compose intent is present. Calling the parent's `onSubmitted` callback as
    soon as the prompt command is created tears that Composer down during the
    originating iOS send gesture, while the keyboard and Page Dock are still
    reflowing; the first tap can then look like it only closed the editor. Keep
    the Composer mounted until the queue/chat acknowledgement succeeds, then
    release the keyboard and clear the Page intent. The send accessory may use a
    stationary touch `pointerup` fallback when WebKit drops the compatibility
    click after scroll momentum, but must retain native textarea focus on
    `pointerdown` and keep ordinary click/keyboard activation intact.

## Verification matrix (run the WHOLE thing after any editor change)

On the **iOS Simulator** (or device), in BOTH the inline composer and the
fullscreen editor:

- [ ] Type pinyin on a `**`/`# ` marker line — no dropped/garbled chars (pitfall #1).
- [ ] Caret is visible + stays visible while typing/scrolling (pitfall #2).
- [ ] With the software keyboard visible, open Clear all, cancel it, reopen it,
      and confirm it: the keyboard stays visible throughout, text clears only on
      confirm, and the same editor retains its caret (pitfall #43).
- [ ] Long-press → the Paste/Select menu appears; Paste works (pitfall #3).
- [ ] In fullscreen, long-press near text and in an empty canvas both use the
      same native iOS edit menu; no Cowboy-drawn Paste pill appears.
- [ ] No stray dot near the caret / rendered markdown (pitfall #4).
- [ ] `**`/`(`/`` ` ``/`[` then Backspace clears the whole pair (pitfall #5).
- [ ] Put the caret/selection in the middle of inline text → expand → type →
      collapse: text and the exact forward/backward selection are preserved,
      with no second focus write (pitfalls #6 and #58).
- [ ] Edit a Queue/Draft row, change text, then tap the panel header: the edit
      stays visible behind a Save/Discard/Keep decision; no hidden editor owns
      the composer slot. Save or Discard collapses the panel and restores the
      ordinary new-message composer (pitfall #17).
- [ ] Start an unchanged Queue/Draft edit and tap the panel header: it closes and
      folds directly without a confirmation flash (pitfall #17).
- [ ] Tap a Queue/Draft message to edit: the compact editor appears with its caret
      at the end and the software keyboard opens in the same interaction
      (pitfall #40).
- [ ] Dismiss the keyboard while editing a Queue/Draft row (inline and fullscreen):
      the latest non-empty buffer persists and the default pending card returns;
      the accessory action shows Hide keyboard rather than Save/Done (pitfall #18).
- [ ] In fullscreen, Hide keyboard only lowers the software keyboard. The
      surface stays open, the slot becomes Edit, and tapping it focuses the
      caret at the end and raises the keyboard. CloseFullscreen (not Hide)
      is what leaves fullscreen (pitfall #23).
- [ ] Toolbar quote/list/heading: caret lands AFTER the marker (pitfall #7).
- [ ] Attach a photo, then type — keyboard returns, input works.
- [ ] Paste a photo in the middle of text — the image lands at the caret and the
      caret resumes immediately after the image, before the original trailing
      text; the keyboard stays visible.
- [ ] **OPEN (pitfall #69):** after a pasted image, software-keyboard Return
      (and Backspace back onto the image) must move the *painted* iPhone
      caret with CM6. Not accepted. Simulator CSS caret is not evidence.
- [ ] With an empty clipboard, the Paste accessory is visibly disabled. Copy
      text and it becomes enabled, replaces the exact current range, leaves the
      caret after the inserted text, and keeps the keyboard open. Copy a photo
      and the same action becomes enabled without reading either payload merely
      because the app focused or resumed (pitfall #59).
- [ ] In the main composer and Queue/Draft editing, test Paste in both compact
      and fullscreen states: copied text or an image replaces the exact current
      range, the keyboard remains open, and neither the button nor `BODY` owns
      focus after insertion. Repeat with an image supplied by the Screenshot/IME
      shelf so the provider data/file fallback is exercised (pitfall #59).
- [ ] Attach a photo, put the caret below it, press the SOFT-keyboard Backspace
      twice — the image is ringed then deleted, the token-free native textarea
      inherits the exact caret, and the keyboard stays open (pitfall #12;
      keydown-only handlers don't fire on a phone, so this is the beforeinput path).
- [ ] Headings/bold/italic/code/list render (inactive line) + reveal (active line).
- [ ] Native iOS Pinyin: type unmarked latin, tap a candidate in the
      system bar — the Chinese word commits and composition continues.
      Repeat after an inline image, and on the native textarea (no
      image / no complete markdown). WeChat IME still works (pitfall #80).

Desktop (Chrome bridge) covers render + vim + desktop IME, but **cannot** prove
#1–#3 — those are iOS-only.

Desktop Vim + IME checks:

- [ ] In Normal and Visual/operator-pending mode, `compositionstart` enters Insert.
- [ ] With a CJK input source active, physical `i` enters Insert without opening a
      candidate window; motions/operators remain Vim commands in Normal mode.
- [ ] `a/A/o/O/s/S/C/R` and `c{motion}` enter Insert with a visible native caret;
      for `o/O`, the Selection anchor is the new active `.cm-line`, not `.cm-content`.
- [ ] `.cm-content` stays the identical DOM node and remains `contenteditable=true`.
- [ ] Composition text is accepted with a visible, focused caret after the switch.
- [ ] Typing/composition keeps the same `.cm-content` node and emits no callback-
      identity `StateEffect.reconfigure` after the document transaction.
- [ ] Escape returns focus to the command sink and the status line shows `IME SAFE`.
- [ ] `Mod+P` focuses and expands a visible Plan from any Agent region;
      bare `p/P` remains native Vim paste in every composer state.
- [ ] At a mobile viewport, the Desktop Vim/IME chunk is not requested and Mobile
      editor behavior is unchanged.

43. **Editor chrome and input actions must not share one mobile overlay rail.** The touch composer reserves its top-right edge only for editor-level chrome: Fullscreen and Clear all. Extending that absolute rail into the delivery and formatting bands makes Force push appear to join the rail and causes the three rows to overlap on narrow phones. Hide keyboard is instead the sticky trailing action of the horizontally scrollable delivery row; it is shown only while the real editor owns focus. Clear all stays visible but disabled when the composer is empty. Its confirmation must use a non-modal `Popper`, not MUI `Popover`: disabling `Popover`'s auto/enforced/restored focus was insufficient because its underlying `Modal` lifecycle still ended the iOS software-keyboard session. Opening, cancelling, and confirming all prevent pointer-down focus transfer; confirmation clears the editor and attachments in the same still-focused editor, with no delayed refocus.

44. **Touch Composer action density follows available tablet width.** Portrait
    phones distribute the compact message actions across the row so every 44pt
    target remains easy to reach. At 700px and wider, including iPad split and
    full-width layouts, the same actions form a compact left-aligned group with
    a small fixed gap. Do not stretch a few actions across an iPad canvas or
    classify iPad as Desktop to obtain this layout; this is a visual density
    rule inside the touch product and does not change editor, focus, keyboard,
    or gesture ownership.

45. **A focused Mobile Composer is owned by the real editor, not by its outer
    card.** The compact card contains utility buttons, so card-wide
    `:focus-within` can remain true after the textarea/contenteditable has
    blurred and leave a tall canvas that looks editable but cannot summon the
    iOS keyboard. Promote the card, formatting row, and keyboard action only
    from `[data-mobile-editor-area]:focus-within`. The token-free native
    textarea must remain content-sized and grow through MUI's native autosizer
    up to its row limit. Do not give its parent an independent focused minimum
    height or force the MUI/textarea chain to `height: 100%`: that manufactures
    a large blank writing canvas below a short prompt. Every *visible* writing
    pixel must still belong to the native textarea; a larger editor is earned by
    wrapped/new lines or the explicit fullscreen surface. Inline-image CM6 keeps
    its resolvable height chain because its contenteditable widget canvas has a
    different ownership model. Do not replace either path with mirrored React
    focus state or delayed refocus. iOS and third-party keyboards may dismiss
    while leaving that real editor focused. Observe the shared
    `visualViewport` keyboard signal and, only after a proven open-to-closed
    transition, synchronously release the surviving Mobile Composer focus.
    This keeps native `:focus-within` authoritative while preventing a hidden
    keyboard from leaving the promoted canvas and toolbar behind. Never blur on
    the initial focused-but-not-yet-resized interval.

46. **Mobile editing ownership and software-keyboard visibility are separate
    layout signals.** Queue/Draft editing says which buffer owns the editor; it
    does not prove that the keyboard is visible. Plan/column layout likewise
    describes surrounding content, not permission to fill the phone viewport.
    Only the shared `keyboardOpen` signal may promote pending-editor height or
    keyboard-focused chrome. Keep `height: 100%`, flex-fill, and editor `fill`
    behavior Desktop-only, otherwise Plan plus Queue/Draft can leave a large
    empty Composer after keyboard dismissal. Desktop embedded controls must
    scale icon and keycap geometry through `--cowboy-font-scale` or root-relative
    units; fixed pixel or minimum-font-clamped glyph sizes drift from the global
    font-size setting.

47. **Queue/Draft editing chrome is keyboard-bound on touch surfaces.** The
    initiating tap may briefly mount the editor before `visualViewport`
    publishes an open keyboard, but once an open frame has been observed the
    first closed frame must render the ordinary pending card immediately.
    Buffer persistence and editing-ownership cleanup may finish in the next
    animation frame; stale ownership must never keep the expanded editor or its
    formatting/delivery rails visible without a software keyboard. Some iOS
    third-party keyboards shrink `innerHeight` and `visualViewport.height`
    together, so keyboard detection needs a keyboard-free baseline plus actual
    editable focus, not only their instantaneous height difference.

48. **Fullscreen delivery closes on acknowledgement, not on intent.** A main
    Composer Send/Queue remains expanded while its authoritative operation is
    pending, then closes only after success. Force push follows the same rule
    after its confirmation action resolves; closing only the confirmation
    Popover leaves an empty fullscreen editor behind. Cancellation, rejection,
    and invalid/no-op delivery retain the editor and its draft so the user can
    retry. Keep Draft/Queue row editing separate: its primary action finishes
    the edit transaction rather than delivering that pending item.

49. **The floating Composer stack has one border-box geometry owner.** Settled
    actionable status, Permission, Plan, Queue/Draft, attachments, and the
    primary Composer stay in one ordinary flex column with one 4px rhythm.
    Transient judge progress is different: it belongs to the live Transcript
    tail and must never mount a Composer stack slot. Its pill shares the settled
    status pill's height and relies on the existing Transcript bottom inset, so
    judge-progress removal and verdict-status insertion exchange the same visual
    band without another reservation or a boundary jump. Settled Status and
    Permission must not return to absolute positioning or publish a root-level
    height variable: separate mount/cleanup timing lets Transcript combine stale
    reservations after context clear, panel editing, and keyboard dismissal. App
    measures the complete stack once and publishes one settled Transcript bottom
    inset.
    Observe the AppBar and Composer with `ResizeObserver`'s `border-box`, not its
    default content box: iOS keyboard dismissal restores safe-area padding while
    leaving the content box unchanged, so the old observer kept a 44px navbar
    reservation after the real outer height returned to 60px. Viewport and focus
    settle measurements remain the final safety net for WebKit frames that omit
    the last resize callback. Every direct Mobile stack child is full-width with
    `min-width: 0`; a long editor or pending row must never transfer its intrinsic
    width to the stack. This changes layout ownership only—do not add editor
    remounts, delayed focus, controlled values, or IME event handling to repair
    geometry. The Mobile Transcript owns one DOM-first 6px tail spacer below
    whichever element is visually last: Markdown, Thinking, Judging, optimistic
    content, quiet recovery, or a Page footer. Keep this external to every item;
    type-specific padding recreates unequal boundaries whenever the final item
    changes. The Composer stack separately retains its 4px internal rhythm.
    Each Thought
    step owns a dedicated indicator grid lane: the bulb/dot centre is anchored to
    `0.5lh`, the inherited first-line centre, while the lane stretches with
    multi-line content for its connector. Do not restore separate current/done
    `em` top offsets; they drift whenever Reading font size or surface padding
    changes. A keyboard-focused Mobile Composer is also one bounded flex column:
    the shell, workspace, card, and editor may shrink with `min-height: 0`, while
    formatting and delivery rows never shrink. A short native textarea remains
    content-sized; only an overlong prompt is clamped by the available WebView
    height and scrolls inside the real textarea. Never cap or scroll the complete
    card, because that hides the rich-text row and moves editor chrome with text.

50. **Queue/Draft keyboard completion must survive native long-press setup.**
    iOS can briefly publish a keyboard-closed `visualViewport` frame while UIKit
    promotes a textarea long press into its Paste/Select menu. Pending-row edits
    used to treat that first false frame as completion, immediately auto-save,
    and unmount the textarea; UIKit then lost its native anchor and only the
    keyboard dismissal remained visible. Keep the real touch editor mounted for
    the shared 550ms keyboard settle window and finish only if the keyboard stays
    closed. A renewed open frame cancels the timer. The explicit Hide keyboard
    action remains immediate. In fullscreen pending-row editing, keep that
    keyboard-only action fixed at the lower track's right edge and use
    `CloseFullscreen` for the upper-right completion action that saves and
    returns to the ordinary card; two KeyboardHide glyphs falsely present
    different actions as duplicates. Do not
    intercept pointer, touch, `contextmenu`, or
    selection events to implement this guard: UIKit must continue to own the
    long-press sequence end to end.

51. **Focused Queue/Draft editors use the primary input's material, not a
    transparent pending-panel fill.** The pending row already shares the native
    textarea and keyboard accessory dock, but its old transparent Paper allowed
    the transcript to remain legible underneath the floating edit surface. That
    looked like editor text colliding with the conversation even though DOM
    geometry was correct. Keep the focused background, border, radius, blur, and
    shadow in `mobileFocusedComposerSurfaceSx` and apply that primitive to both
    the primary input and pending-row editor. The fill is fully opaque paper:
    CodeMirror's scroller is a separate iOS compositor layer, so any alpha
    still samples the transcript. Apply the same fill whenever the software
    keyboard is open, not only while `:focus-within` holds. This is a
    paint-only alignment: do not add focus handlers, editor remounts,
    controlled values, or custom touch handling.

52. **Touch-only pointers must not retain synthetic-hover tooltips.** MUI's
    `disableTouchListener` prevents its long-press path, but iOS WebKit can still
    synthesize mouse hover after tapping an icon and leave the tooltip visibly
    stuck. Keep focus and touch listeners disabled globally, and also disable
    the hover listener when `(hover: hover)` does not match. Real desktop hover
    keeps tooltips; touch controls remain named by their `aria-label`. Do not
    solve this with pointer/touch interception around the composer because that
    would compete with UIKit's native selection and paste sequence.

54. **Mobile Queue/Draft inline edits expose context actions in the first
    accessory track.** Keep `Expand editor` in that track's non-scrolling right
    group so it remains stable while Attach and contextual actions scroll; never
    put it back as an absolute button over the writing canvas. Draft editing
    exposes Send and Schedule; Queue editing exposes confirmation-gated Force push. Persist the
    latest editor buffer before any of these operations, close only after an
    authoritative delivery acknowledgement, and retain the editor on rejection.
    Scheduling may release the editor before opening its sheet because it commits
    the durable draft first. Keep every control's pointer-down focus protection;
    do not add custom touch, selection, or IME handling.

55. **Touch textarea value ownership must stay native across selection changes.**
    A React-controlled `value` can replace WebKit's live selection when iPad
    long-press publishes `select` and the parent re-renders before UIKit presents
    Paste/Select. Use `defaultValue` for the native textarea, mirror input to
    React without writing it back on every render, and synchronously apply only
    genuine external replacements with a mapped selection. Toolbar edits,
    undo/redo, and native-to-CM6 image promotion must write their value and caret
    in the same gesture; never restore them in a later animation frame. This
    preserves native paste/IME ownership and prevents repeated newline input from
    appearing to jump until the next character is typed.

56. **Popover image deletion must remove the same block that insertion created.**
    Image placement adds a line break before and after the token when it is
    inserted in the middle of text. Delete actions must consume that surrounding
    one-line range and map the old selection through it; removing only the token
    line leaves a stale separator and makes the caret appear on the wrong line.

57. **Queue Force push confirmation must not unmount its own anchor.** MUI
    `Popover` is backed by `Modal`; opening it moves focus out of the native
    textarea, dismisses the iOS keyboard, and completes the keyboard-bound Queue
    edit. The accessory button is then unmounted, so Popover loses `anchorEl` and
    falls back to the viewport's top-left under the status bar. Keep this confirm
    on non-modal `Popper` with click-away dismissal, prevent pointer-down focus
    transfer on both confirmation buttons, and close the editor only after the
    authoritative Force push acknowledgement. Do not repair the symptom with a
    safe-area offset: the missing anchor and lost edit transaction are the bug.

58. **Compact/fullscreen replacement must transfer the logical selection, not
    call `focusEnd()`.** Main and Queue/Draft expand/collapse used to preserve the
    document but place the replacement editor's caret at its end. The pending-row
    expand path also focused the new editor once from its layout effect and again
    after `flushSync`; that second selection write could leave an iOS caret with
    no keyboard. Both native textarea and CM6 handles expose `{anchor, head}` and
    restore it synchronously in the originating accessory gesture, including a
    backward selection. Pointer-down on the compact fullscreen control must not
    focus its button first. Initial Queue/Draft edit may still deliberately use
    `focusEnd()`; it has no replaced user selection.

    Image insertion follows the same range contract. Native textarea and CM6
    both replace the current selection, sanitize the token label, and land after
    the complete image batch. Queue/Draft paste must stage local image
    placeholders and insert their tokens synchronously before raster encoding or
    a native payload read, just like the primary Composer; delivery, persistence,
    and keyboard-dismiss
    completion remain disabled while any placeholder is pending. Waiting for
    encoding before the first token moves native-to-CM6 promotion outside the
    UIKit Paste gesture and loses its caret/keyboard transaction. When paste
    batches overlap, each completion may replace or remove only the placeholder
    ids it created; treating every `pending` attachment as part of the completed
    batch drops newer images and leaves orphaned document tokens.

59. **The dedicated Mobile Paste action is a native capability, not a
    browser clipboard menu.** Keep the action fixed immediately after Undo/Redo
    in the shared formatting row, independent of the user's configurable toolbar,
    so main, Queue, and Draft editors expose it in both compact and fullscreen
    states. The iOS shell probes `UIPasteboard.hasStrings`, `hasImages`, plus
    loadable `NSItemProvider` text/URL/attributed-text capability and
    loadable/declared provider image capability, image count, and change count
    to render the exact enabled state without reading a payload. Images
    take precedence when the pasteboard advertises both forms; otherwise text
    replaces the captured logical range and leaves a collapsed caret after it.
    Provider fallback is required because some source apps publish copied text
    or photos lazily rather than exposing an eager `UIPasteboard.string` or
    PNG/JPEG representation. On the explicit read, load provider-backed
    `NSString`, attributed text, or URL as plain text; for images, fall back
    from `UIImage` loading to registered image data and file representations.
    Capability without retrievable content is not a successful paste. Current
    shells expose exact text metadata. A legacy shell that has the explicit
    text-read bridge but predates `hasText` keeps Paste enabled in an
    availability-unknown compatibility mode; an empty read must leave the
    selection untouched. Shells without a read bridge and ordinary browsers
    keep the action disabled. Do not restore a
    privacy-gated `navigator.clipboard.read()` preflight or infer availability
    from a previous paste. Read text or encode image payloads only after the
    explicit tap.

    The accessory primitive prevents pointer-down focus transfer. Snapshot the
    editor's logical `{anchor, head}` on that pointer-down. iOS WebKit
    accessibility activation can still focus the button and retarget the later
    compatibility `click` to an ancestor, so commit a stationary touch on the
    button's reliable `pointerup` path and suppress its duplicate click; a
    click-only handler can dismiss the keyboard without ever starting paste.
    Synchronously insert pending tokens for the advertised image count against
    the captured range, and commit the native
    textarea -> CM6 focus transfer **before invoking** the privacy-gated payload
    bridge. The native textarea must own focus during that value/selection write
    so the open software keyboard transfers with the caret. Resolve those exact
    pending ids after bytes arrive; a stale count may add/remove only its own
    batch. Pasteboard-change,
    app-resume, visibility, and window-focus notifications refresh metadata, but
    iOS pasteboard notifications can be process-local: an image copied in another
    app, an IME clipboard shelf, or Screenshot may produce no foreground event.
    While the action is mounted and the document visible, poll only the metadata
    bridge at a restrained interval so the disabled state converges. Neither an
    event nor that poll may read payloads, mutate the document, or refocus the
    editor. Text paste keeps the same editor mounted. Cache its last native
    selection so a click-only accessibility activation cannot replace the
    logical range with WebKit's post-blur `0...0`. Capture that remembered range
    and prevent the click's default focus action, then restore it synchronously
    before asking the native bridge for text. After a reliable touch pointerup,
    iOS may synthesize a separate compatibility `mousedown` even when its later
    click is retargeted outside the button; the shared accessory primitive must
    prevent that mousedown default as well as pointerdown or it will focus the
    button and close the keyboard after insertion. Do not disable the still-focusable
    Paste button while a text read is pending; guard duplicate reads internally
    instead. After the reply, replace that exact range rather than using the
    current or end position.

60. **Top-anchored Mobile Snackbars must clear the status-bar safe area.** MUI's
    narrow-screen default is `top: 8px`; on an iPhone that places the complete
    Move-draft Undo toast behind the status bar and leaves only a blank bottom
    shadow visible. Offset the toast by `safe-area-inset-top`, constrain it to the
    viewport's 8px horizontal gutters, and keep Undo available for the full
    acknowledgement window. This is distinct from a Popover that lost its anchor:
    the Snackbar is healthy and needs safe placement, not focus or anchor repair.

61. **Every paired Mobile action rail shares one right-edge fixed-slot geometry.**
    The upper message track once used symmetric horizontal padding while the
    keyboard-nearest editing track ran to the panel edge. Their otherwise
    identical 48px trailing actions therefore drew vertical dividers several
    pixels apart. The compact composer's absolutely positioned fullscreen action
    repeated the same drift by using its own two-pixel right inset. Keep every
    paired trailing action inside the same fixed-slot primitive, pin an overlay
    slot directly to the card edge, and let flowing upper tracks use left padding
    only. The compact overlay is a lone fullscreen control rather than one half
    of a paired rail, so it keeps the shared 48px geometry but draws no vertical
    divider; only expanded two-track docks need the aligned separators. This is
    layout-only: do not move focus ownership, pointer-down
    prevention, or keyboard actions to repair divider alignment.

62. **Touch session rows must not retain synthetic hover or focus paint.** iOS
    WebKit can synthesize `:hover` after a finger touches a MUI `ListItemButton`;
    the resulting gray band can remain on a non-selected session and compete
    with the real selected row. Mark rows activated by a touch pointer and
    neutralize only their latched hover/focus-visible background. Preserve the
    primary-tinted selected material, native `:active` feedback, real mouse
    hover, and keyboard focus. A real mouse enter or keyboard event clears the
    touch marker. Do not intercept the touch sequence or disable ripple/selection
    semantics to repair paint.

63. **The touch editor must be a literal textarea, not MUI
    TextareaAutosize.** iOS renders a textarea caret as a UIKit-owned selection
    overlay. MUI 7's TextareaAutosize wraps every input event and, when the value
    ends in a newline, calls `setSelectionRange(end, end)` even though the native
    selection is already there. That workaround was added for a Chromium paste
    bug; on a physical iPhone it races UIKit after Return and leaves the painted
    caret on the previous line until an ordinary character causes another native
    update. `overflow:hidden` and making `clientHeight === scrollHeight` do not
    remove this second selection transaction. Render the touch editor directly
    as `<textarea>` and use MUI Box only for theme-aware styling. Keep it
    uncontrolled during typing, use one static `rows` value, and let the focused
    writing canvas provide its height. While the field is focused, never reset
    `style.height` to `auto` to remeasure: that collapse/grow cycle is the same
    class of layout race as TextareaAutosize and leaves the painted caret on the
    previous line after a few Returns. Grow height monotonically while focused;
    fit exactly only after blur. That blur path must first reset the inline
    height to `auto` and only then read `scrollHeight`: clearing a long prompt
    while it is still focused intentionally preserves the tall box, and reading
    `scrollHeight` before releasing that box can measure the stale height again.
    Blur does not itself trigger a useful `ResizeObserver` update, so settle the
    exact height synchronously in the native blur handler. External document
    replacement and explicit toolbar edits may still map a selection
    synchronously; ordinary keyboard input must never call `setSelectionRange`,
    resize the DOM from an input callback, insert a zero-width sentinel,
    blur/refocus, or draw a fake caret.

64. **Do not change touch image decoration structure without physical paste
    acceptance.**
    **OPEN TODO: the current ledger is pitfall #69.** The paragraphs below
    are historical session notes, not a plan to re-apply.
    Touch documents with an image token intentionally promote to CM6 so the
    thumbnail stays in the writing flow. A `Decoration.replace({ block: true })`
    removes that document line's ordinary `.cm-line` and places the image widget
    directly under `.cm-content`. Physical iOS WebKit may then keep its native
    caret on an earlier empty line after Return, even though CM6's logical
    selection and `.cm-activeLine` advanced. Physical telemetry proved that the
    failure is deeper than a stale Selection node: after consecutive Returns,
    Selection was already collapsed at offset zero of the correct active line,
    but its native collapsed Range still had a negative top and zero height.
    Simulator reproduced the same state. The root cause is WKWebView retaining
    the preceding root block replacement's native caret geometry until a
    measurable text glyph causes another layout. A root-node-only Selection
    workaround was therefore structurally incomplete.
    An attempted repair changed touch to a non-block replacement nested in an
    ordinary `.cm-line`. Simulator paste and HID Return passed, but physical
    iPhone acceptance immediately regressed image paste. Merely changing that
    experimental presentation facet back to `true` was not a complete rollback:
    it left the alternate facet, widget constructor, and reconfiguration-aware
    StateField path installed, and physical paste still failed while Simulator
    paste passed. Remove the presentation branch entirely and keep the literal
    `Decoration.replace({ block: true })` field that existed before the
    experiment. Keep this proven block replacement on touch and Desktop until a
    replacement design passes the
    complete physical sequence: first image paste, second image paste while CM6
    is already mounted, permission-alert return, and trailing Return before any
    typing.

    Physical v1243–v1247 proved two negative results. (1) A one-paint
    editable U+200B widget measures the native caret (`caret_height=12`)
    only while it exists; after removal the Range returns to height 0.
    (2) Nesting the image in an ordinary `.cm-line` (`block: false`) still
    leaves `caret_height=0` and a negative `caret_top` on Return and on
    Backspace. The failure is therefore "empty line after a replaced image
    widget", not "root-level versus nested block".

    Keep the proven `Decoration.replace({ block: true })` on touch and
    Desktop. Physical v1250 showed the late-mount repair was the wrong
    trigger. After paste the landing line still had no text node, so first
    Return left the UIKit caret on the thumbnail (`caret_height=0`). The
    widget then appeared on the *next* empty line and ate the second Return
    as a native `<br>` (`line_height` 14→28, `document_lines` unchanged for
    ~280ms). Backspace onto that same landing line dropped the widget on
    `docChanged` and remounted it too late. The failure is specifically
    "empty line whose previous line is a block image", not every empty line.
    The image is not a `.cm-line`. `Decoration.replace({ block: true })`
    lifts the token out of the line flow, and `inlineImageTrailingLine`
    always keeps a second empty landing line so the caret is not trapped
    on the atomic thumbnail. After paste the document is already two
    lines that look like one image. Physical v1251 put a landing U+200B
    on that empty line and first Return finally moved the UIKit caret —
    but only by inserting a native `<br>` into the same landing node
    (`document_lines` stayed 2, `line_height` 14→28). A second Return
    stacked another `<br>` (`line_height` 42) while UIKit stayed in that
    node. On touch only, decorate the landing line while the caret is
    actually on it, including paste. When Return fires inside that node,
    preventDefault and insert a real CM6 newline so the caret leaves the
    landing line; then drop the widget. Do not keep the landing widget
    after the caret has moved — that is the magnet. When the widget first
    appears, map the DOM selection into it so the next Return is native
    input inside it. Hide the widget for IME. The image widget's vim
    probe must stay non-selectable. This is not accepted until physical
    first and second Return after paste, Backspace-up onto the landing
    line, second paste, long-press menu, and IME all pass.
    Physical v1253 debug proved `beforeinput.target` is `.cm-content`
    (`content_root`), so an `event.target` closest-to-widget check never
    preventDefaults. First Return then inserts a native `<br>`
    (`line_height` 14→28, still two lines). The next Return advances CM6
    while `caret_height` stays 0. Decide from EditorState — empty line in
    the image chain — and write a real CM6 newline with a destination
    U+200B in that same transaction.
    Physical v1255 then bounced twice on one Return: beforeinput wrote
    `\n` (`document_lines` 2→3) and iOS still delivered `keydown Enter`
    ~250ms later (`3→4`). Consume that trailing Enter for 500ms after a
    materialized image-chain line break. Do not leave the second insert
    to `defaultKeymap`.
    Physical v1256 made the document 1:1 (`mobile_caret_line_break_consumed`
    fired, no second `cm6_doc`). The remaining bounce was the landing
    remap 27ms later: `cm6_doc` was already `caret_anchor`, then
    `placeLandingSelection` called `removeAllRanges`. Skip that remap
    after our own newline, and no-op when the caret is already in the
    widget.
    Physical v1257 still bounced twice with no extra remap. Sequence:
    `cm6_doc` 2→3, then 242ms later `consumed` with no second document
    line. The second motion is UIKit moving on the delayed software-
    keyboard Enter after we already inserted on `beforeinput`. Do not
    write `\n` from `beforeinput`. PreventDefault the native `<br>`
    only, and let that single later Enter insert the line.
    Physical v1258 logs were enough: one keydown Enter = one `cm6_doc`.
    First Return stayed `caret_height=12` at the old `line_top`. Then
    the abandoned landing line grew 14px (`96.5→110.5`) and the native
    Range died (`caret_height=0`, `caret_top=-260`) while later Returns
    still advanced `.cm-activeLine`. Keep the landing U+200B after the
    caret leaves so that line never collapses, and reuse the widget DOM
    (`eq()` true) so a rebuild does not destroy the Range.
    Physical v1259 still died on the second Return. Two widgets were
    present (`caret_anchor_widgets=2`) but `eq()` returned true for
    every instance, so CM6 moved the landing node onto the new line.
    The landing line still collapsed. Key `eq()` by document position,
    and after a newline `Selection.collapse` into the *new* line's
    widget in the same view update — no `removeAllRanges`.
    Physical v1260: first Return kept `line_top` at 96.5 with
    `caret_anchor_widgets=1`. 1.3s later the second widget appeared,
    `line_top` jumped to 110.5, and the Range died until the next
    keydown collapse repaired it. Do not keep a U+200B on abandoned
    landing lines. Give `.cm-inline-image-widget + .cm-line` a 14px
    min-height so that row cannot collapse.
    Physical v1261 proved the earlier "landing height" story was not
    the Return stutter. First Return: `beforeinput insertLineBreak`
    `default_prevented=false`, `line_height` 14→28 (native `<br>`),
    then CM6 wrote `\n`. The plugin handler is too late; a `.cm-widgetBuffer`
    also sits between the block image and the landing line so the CSS
    sibling rule never applied. Prevent the native break on capture,
    and target `widget + .cm-widgetBuffer + .cm-line`.
    The actual mismatch is the image line itself. A block widget at
    `line.to` is still a visual break that is not a document newline.
    Physical v1263 showed the raw token, then first Return wrote a
    native `<br>` into that source line (`line_height` 14→28→42) and
    killed the UIKit caret (`caret_height=0`) while CM6 advanced.
    Replace only the token with an inline widget inside the `.cm-line`
    (same class as `@` chips). Physical v1264 still died: paste forced a
    following empty line (`document_lines=2`, `caret_height=0` before
    Return), then insertLineBreak wrote a native `<br>` (`14→28`).
    Physical v1265 put that space on the image line. The caret became
    an 88px bar beside the thumbnail, and first Return still wrote
    `<br>` into that tall line (`88→102`) before the native caret
    died. Keep the thumbnail on its own line; put the space on the
    next line so the caret is a 12px bar. PreventDefault native
    insertLineBreak on the image line or the line after it, and let
    the single later Enter insert the CM6 newline. Do not show
    cowboy-att markdown. v1268 followed Obsidian's source-line
    reveal; that flashes `![](cowboy-att:…)` in a chat composer and
    was withdrawn.


65. **A reliable touch action must pair `pointerup` with its actual synthetic
    `click`, never a timeout guess.** Mobile Safari can omit a click after
    stopping scroll momentum, so Cowboy commits stationary touch actions on
    `pointerup`. It can also delay the eventual synthetic click for longer than
    the old 700ms suppression timer while WKWebView scroll, keyboard, or layout
    work settles. Letting that timer expire turns one physical Send into two
    activations with different cmids and therefore two identical user bubbles.
    Keep the suppression claim until the next touch-owned or non-zero-detail
    click actually consumes it; a new pointerdown resets the claim, while
    keyboard and assistive clicks (`detail === 0`) remain native. The Composer draft
    controller also claims a commit synchronously for the current browser task
    so its editor-submit and toolbar-submit entry points cannot both mint a
    message before React applies the clear. Recommended Agent preset cards in
    the scrollable Session settings sheet use this same primitive: a user can
    reach them while sheet momentum is settling, where a click-only
    `ButtonBase` otherwise appears to ignore the first tap.

66. **A visible native keyboard is stronger evidence than transient DOM focus.**
    WKWebView can briefly move `document.activeElement` to `body` while UIKit's
    keyboard and caret remain visible, especially across attachment and IME
    transitions. `:focus-within` therefore cannot decide whether the focused
    composer owns its opaque writing material or whether a fullscreen native
    composer should remove the home-indicator inset already consumed by the
    resized WebView. Use the settled `visualViewport` keyboard state for those
    presentation decisions. Keep editor-specific expansion and selection rules
    focus-gated where they truly require an editable descendant; do not infer a
    keyboard from focus alone.

67. **iOS Simulator cannot accept the image-adjacent empty-line caret.**
    Physical v1252 and Simulator HID Return produce the same *document*
    telemetry after paste: first Return stays on two lines with
    `line_height` 14→28 and `caret_height=0` (a native `<br>` inside the
    landing node); the next Return advances CM6 onto a later empty
    `.cm-line` whose Range is still height 0. The Simulator screenshot
    still shows a purple caret on that empty line because Simulator
    WKWebView paints CSS `caret-color` from the focused contenteditable
    selection. A physical iPhone paints a UIKit `UITextSelectionView`
    overlay from `caretRect(for:)`, which keeps the last measurable text
    rect — the thumbnail / landing glyph — when the new line is only a
    `<br>`. HID `axe key 40` also never matches the software-keyboard
    path: `visualViewport` stays full-height, and `beforeinput.target` is
    `.cm-content`, not the landing node. A Simulator purple caret is
    therefore not evidence. The device-faithful signal is
    `caret_height=0` plus the user's painted caret. Do not treat
    Simulator HID Return as acceptance for this bug.

    Simulator and a physical iPhone will never paint the same caret.
    Alignment means forcing Simulator onto the *software-keyboard input
    path*, then judging it with the same telemetry the device emits —
    not making the purple CSS caret match UIKit.

    Simulator alignment protocol (this is the only way to get closer):

    1. **Disconnect the Mac keyboard.** Simulator → I/O → Keyboard →
       uncheck **Connect Hardware Keyboard**. `⌘⇧K` toggles it. If this
       stays on, iOS treats the Mac keyboard as a paired hardware
       keyboard: no software keyboard, no `insertLineBreak`, no
       `visualViewport` shrink, and Return is a HID `keydown Enter`.
    2. **Show the software keyboard.** `⌘K` / **Toggle Software
       Keyboard**. Tap the composer so the keyboard is actually
       on-screen. Paste the image with the long-press menu, not a Mac
       paste shortcut.
    3. **Tap 换行 on that software keyboard.** Do not press the Mac
       Return key and do not send `axe key 40`. The faithful sequence is
       `beforeinput insertLineBreak` then a later `keydown Enter`, with
       `software_keyboard=true` and `enter_path=software`.
    4. **Reject the run if debug says otherwise.** In Debug mode,
       `enter_path=hardware_or_hid` or `software_keyboard=false` on
       Return means the session is still the Mac-keyboard path. That
       run cannot accept or refute the physical caret bug.
    5. **Trust `caret_height` and the user's painted caret, not the
       Simulator screenshot.** A 12px CSS caret on Simulator can sit on
       a line whose UIKit `caretRect` is 0. Physical iPhone is still
       the acceptance surface.

    There is no WebKit flag, CSS property, or CM6 option that makes
    Simulator's CSS caret become `UITextSelectionView`. Product code
    must not try to emulate UIKit in CSS to "align" the two.

68. **Composer debug mode is the physical-input flight recorder.** Settings
    → Debug mode (Desktop and Mobile) turns on `composer_input_debug`
    samples. They ride the existing `/api/observability/batches` path into
    VictoriaLogs. Samples record CM6/DOM geometry, `inputType`, target
    relation, and viewport height — never document text, key characters,
    clipboard contents, or attachment ids. Off by default. Query
    `event_name:="composer_input_debug"` or `event_name:="composer_debug_mode"`.
    This does not make Simulator match a physical caret.

69. **TODO (OPEN): physical iPhone caret after a pasted composer image.**
    **Do not claim this fixed.** Status is OPEN as of cowboy-v1269 /
    `a9aaa415` (2026-08-15). Physical iPhone acceptance only. Simulator
    CSS caret and HID Return are not evidence (pitfall #67). Debug mode
    (pitfall #68) is the flight recorder. Historical session notes that
    led here live in pitfall #64; they are not a plan to re-apply.

    ### Reproduction (physical iPhone only)
    1. Mobile composer, software keyboard visible, Debug mode on.
    2. Long-press paste a photo (not a Mac / HID paste).
    3. Tap Return on the software keyboard (换行), not a hardware key.
    4. Repeat Return. Type a letter. Backspace back onto the image.
    Expected: the *painted* caret moves with CM6 on every Return and
    Backspace. Actual: CM6 selection / `.cm-activeLine` advance, the
    UIKit caret stays on the thumbnail or the last measurable rect
    until a normal character. The image is required; text-only Return
    is fine. `@` chips are fine.

    User-reported shapes of the same bug (all still this item):
    first Return bad / second good; first good / second bad; one
    Return bounces twice; Return lags; only text at the end of an
    image; Backspace will not walk the caret back up onto the image.
    2026-08-15 IME split (user, not yet agent-probed on a live
    device): native iOS Pinyin, first Return a bit wrong; WeType,
    first few Returns wrong. Same Return-after-image shape in
    Obsidian.

    ### Current code (cowboy-v1305+)
    Tray-only touch paste (v1304, candidate 2) was withdrawn: it
    bypassed the inline image instead of fixing the caret. Token is
    again `![name](cowboy-att:<id>)`. Touch promotes to CM6 on the
    first token. The token is replaced by an inline thumbnail widget
    (not `block: true`). Paste inserts a real following line with a
    trailing space and lands the caret there:

    ```
    lead + token(s) + "\\n " + optional trail
    caret = after the space on the next line
    ```

    Image-adjacent `insertLineBreak` is preventDefaulted; landing
    U+200B widgets are inert. Obsidian source-line reveal (`v1268`)
    was withdrawn because flashing `![](cowboy-att:…)` is too clumsy
    for chat.

    Primary files: `web/src/inlineImages.ts`,
    `web/src/inlineImageSelection.ts`,
    `web/src/composer/inlineImageCaretPolicy.ts`,
    `web/src/composer/mobileEmptyLineCaret.ts`,
    `web/src/composer/mobileLineBreakCaretTelemetry.ts`,
    `web/src/composer/composerInputDebug.ts`,
    `web/src/ComposerEditor.tsx`,
    `web/src/composer/PlatformComposerEditor.tsx`.

    ### Hard constraints
    No sentinel in EditorState / React / history / persistence /
    messages. No fake caret / `drawSelection`. No ordinary
    `setSelectionRange` / focus / blur loops. Keep iOS native paste,
    selection, long-press, and IME. Desktop must stay usable. Do not
    treat Simulator HID Return as acceptance.

    ### Proven facts (physical Debug logs + user)
    - After paste+Return, JS Selection can already be collapsed at
      offset 0 of the correct active `.cm-line` while the native
      Range has `caret_height=0` and a negative `caret_top`.
    - Software-keyboard Return is `beforeinput insertLineBreak` then
      a delayed `keydown Enter` (~250ms). HID / Simulator hardware
      Return is only `keydown Enter`.
    - When `insertLineBreak` is not preventDefaulted, `line_height`
      grows 14→28 or 88→102 (`<br>` in the current node) while
      `document_lines` stays put until CM6 later writes `\n`.
    - `beforeinput.target` is always `.cm-content`, never the widget.
    - A landing U+200B measures `caret_height=12` only while it is
      mounted; after removal the Range returns to height 0.
    - Nesting the image in an ordinary `.cm-line` (`block: false`)
      does not by itself give UIKit a live caret.
    - A touch-only decoration facet passed Simulator and broke
      physical paste. Facet was removed.
    - Writing `\n` from `beforeinput` double-inserts when the delayed
      Enter arrives. Consuming that Enter makes the document 1:1;
      UIKit still bounces.
    - `eq()`-always-true landing widgets migrate onto the new line.
    - Trailing space *on the image line* paints an 88px caret bar.
    - Obsidian reveal shows the raw token; user rejected it.
    - Simulator can match document telemetry and still paint a
      purple CSS caret. That is not the physical bug.
    - 2026-08-15 user: Obsidian fails the same way. Native Pinyin
      misbehaves on the first Return; WeType on the first few.
      Leading hypothesis, not a close. WeType is not installable on
      the iOS Simulator (no App Store; Mac WeType is desktop IMK).

    ### Best current diagnosis (not a shipped fix)
    Two layers. The IME layer is a 2026-08-15 user report and is not
    yet re-probed with Debug logs.

    **IME (leading new hypothesis).** After an image, WeType (and to a
    lesser degree native Pinyin) likely spends the first Return or
    first few Returns on candidate confirm / bar dismiss instead of a
    clean newline. That matches the native-vs-WeType severity split
    and the same failure in Obsidian. It does not by itself explain a
    painted caret that stays on the thumbnail after CM6 /
    `.cm-activeLine` have already advanced.

    **UIKit geometry (earlier physical logs).** Physical iPhone paints
    `UITextSelectionView` from `caretRect(for:)`. After an image
    widget (a replaced `<img>`, plus CM6's empty
    `<img class="cm-widgetBuffer">`), that rect binds to the last
    measurable replaced box. Software-keyboard `insertLineBreak`
    writes a native `<br>` into the current contenteditable node.
    CM6 then writes a document `\n` and remaps JS Selection. UIKit
    does not remount onto the new `.cm-line` until a real character
    creates a text node (`caret_height` returns to 12). Simulator
    still paints CSS `caret-color` from the JS Selection, so it looks
    fine.

    These can both be true: IME consumes or delays early Returns,
    then the remaining `insertLineBreak` still hits a dead
    `caretRect`. Do not drop the geometry facts. Do not treat
    "WeType did it" as a shipped fix.

    `@` chips work because they are text-height, not a tall `<img>`.

    The user's guess ("the image looks like a line but is not") was
    half-right for the old `Decoration.replace({ block: true })` path,
    which lifted the token out of `.cm-line`. After v1264 the token
    is a real source line with an inline widget, and the caret still
    dies.

    ### What others do
    Obsidian (same CM6 stack) hits the same bug. Staff called it an
    upstream CodeMirror problem. Their workarounds: Source mode, tap
    the image to reveal `![]()` then Return from that text, toolbar
    "move line". Changelog 1.4.12 only says it was "sometimes" fixed.
    Chat apps (iMessage / Slack) keep images *outside* the text field.
    CM6's recommended escape is `drawSelection`, which Cowboy cannot
    use (it kills the iOS long-press paste menu). No public write-up
    shows a working native caret next to an in-flow CM6 image widget
    on physical iOS.

    ### Debug
    Settings → Debug mode. VictoriaLogs:
    `event_name:composer_input_debug`. Useful fields: `build`,
    `input_type`, `document_lines`, `state_line`, `line_height`,
    `caret_height`, `caret_top`, `default_prevented`,
    `previous_line_is_image`, `has_image_widget`, `software_keyboard`,
    `enter_path`. Reject a Simulator run when `enter_path` is
    `hardware_or_hid` or `software_keyboard` is false on Return.

    Signature that the bug is still open: after software-keyboard
    Return next to an image, `state_line` advanced (or
    `.cm-activeLine` moved) and `caret_height` is 0 or `caret_top`
    is stuck / negative, while the user still sees the caret on the
    thumbnail. A log with `caret_height=12` is not acceptance if the
    user says the painted caret did not move.

    ### Failed attempts (do not repeat without new physical evidence)

    | Ship | Attempt | Physical result |
    |---|---|---|
    | ~v1240 | Treat paste/caret as already fixed | User: not fixed |
    | v1243–v1247 | Transient editable U+200B; nest widget in `.cm-line` | Measures only while mounted; Return/Backspace still `caret_height=0` |
    | facet / `inlineImagePresentation` | Touch vs Desktop decoration branch | Physical paste broke; Simulator passed. Branch removed |
    | v1250 | Late-mount landing U+200B | First Return still on thumbnail; second Return ate `<br>` |
    | v1251 | Persistent landing U+200B | First Return moved only via native `<br>` (`14→28`); second stacked `<br>` (`42`) |
    | v1253 | preventDefault from `event.target` closest-to-widget | `target` is always `.cm-content`; never fired |
    | v1255 | Write `\n` from `beforeinput` | One Return → two `cm6_doc` (beforeinput + delayed keydown Enter) |
    | v1256 | Consume trailing Enter 500ms | Document 1:1; remaining bounce was `removeAllRanges` remap |
    | v1257 | Skip remap after our newline | Bounce remained: UIKit still moves on delayed Enter |
    | v1258 | preventDefault `<br>` only; let later Enter insert | First Return `caret_height=12` at old `line_top`; abandoned line grew; Range died |
    | v1259 | Keep U+200B after leave; `eq()` always true | Second Return: widget moved onto new line; landing collapsed |
    | v1260 | Position-keyed `eq()` + `Selection.collapse` | Late second widget 1.3s later; Range died |
    | v1261 | CSS min-height on `widget + .cm-line` | Selector missed `.cm-widgetBuffer`; stutter was native `<br>` |
    | v1262 | Capture-phase preventDefault + buffer sibling CSS | First Return still 2→3 with `line_top` stuck |
    | v1263 | Hang block widget at `line.to`; show token when active | Raw token visible; Return wrote `<br>` into source line `14→28→42` |
    | v1264 | Inline replace; caret on following empty line | Caret already dead after paste; Return `<br>` `14→28` |
    | v1265 | Trailing space on the image line | 88px caret bar; Return `<br>` `88→102`; then `caret_height=0` |
    | v1266 | Space on the *next* line + preventDefault adjacent Return | Telemetry sometimes kept `caret_height=12`; user: Return still wrong |
    | v1268 | Full Obsidian source-line reveal | User: too clumsy for chat. Withdrawn in v1269 |
    | v1304 | Keep new touch images in the tray; never promote | User: bypass, restore inline |
    | event patches in general | consume Enter, remap DOM Selection, focus/blur | Double bounce, dead Range, or paste-menu regressions |

    ### Do not retry
    `drawSelection` / fake caret. Sentinels in EditorState. Ordinary
    `setSelectionRange` / focus / blur. Claiming Simulator HID Return
    as success. Re-shipping Obsidian token reveal. Another
    beforeinput/`<br>` intercept without a new structural reason.
    Another landing U+200B `eq()` / remap tweak. Another
    "already fixed" close-out without physical user confirmation.

    ### Next candidates if this is reopened
    1. Physical iPhone probe with Debug mode: same paste+Return
       sequence on WeType, native Pinyin, and English. Count Returns
       until `document_lines` / painted caret move. Log composition /
       `insertFromComposition` vs `insertLineBreak`. Simulator WeType
       is not a substitute.
    2. Keep images *out* of contenteditable on iPhone (attachment
       row / tray; textarea stays the editor). This is what native
       chat composers do.
    3. A widget that is not an in-flow `<img>` (and that does not
       leave `cm-widgetBuffer` as the UIKit caret box), only if a
       new physical probe shows `caretRect` bound to that `<img>`.
    4. Do not start another input-event patch unless that physical
       IME probe gives a new structural reason.

    Re-open only with a new structural hypothesis and a physical
    iPhone probe. Rebase `origin/main` before any next caret change.

    ### Acceptance (unchanged)
    Physical iPhone, Debug mode on: paste image, first and second
    Return, type after the image, Backspace onto the image, second
    paste, long-press menu, IME. All must pass. User confirmation
    required. Never close this item from Simulator, telemetry
    alone, or an agent "it should be fixed now".

70. **A second dedicated Paste after an inline image must not replace
    the first thumbnail.** After the first paste the compact editor is
    already CM6. iOS often reports the first image's atomic range, or
    the whole document, as the captured selection. Staging a pending
    token into that range then settling an empty/failed payload deletes
    the only remaining token — a flash of a picture, then none.
    `inlineImagePasteInsertion` inserts after any overlapped
    `cowboy-att:` token instead of substituting it. The CM6 mount seed
    still ignores ordinary typed text (so @uiw cannot bounce IME), but
    it must follow a changed image-token set; otherwise a later React
    `value` sync resets the live doc back to the first-image seed and
    the second thumbnail disappears after one frame. This does not
    claim pitfall #69 fixed.

71. **A long inline-image prompt needs its own bounded editor scroll
    viewport when the software keyboard opens.** Keeping `.cm-scroller`
    at `overflow: hidden` clips the live selection under the fixed
    composer actions once several thumbnails make the document taller
    than the keyboard-open canvas. Keep the outer composer shell and
    editor wrapper clipped, but allow only the focused CM6 scroller to
    scroll within a viewport-height bound. On the closed-to-open
    keyboard transition, reveal the current CM6 selection without
    changing focus or selection; the native textarea implementation is
    intentionally a no-op so UIKit continues to own its selection and
    IME. This is a viewport-reveal fix only and does not claim the
    physical painted-caret behavior in pitfall #69 is fixed.

72. **A Queue/Draft kebab or Force tap must not start an edit.** The
    pending card's empty fill is an edit hit target (`pendingEditTap` on
    the preview box, not the whole Paper). iOS still delivers a
    bubbling `click` / `pointerup` after a nested `IconButton`, and a
    dismiss of the More menu can ghost-click the card underneath.
    Keep the tap target on the preview/attachments only, ignore the
    same nested-control selector as `useReliableTouchTap`, suppress
    edit for one gesture after the kebab opens or closes, and do not
    use `cursor: text` on the unread-only card. This does not claim
    pitfall #69 fixed.

73. **A transient Desktop Vim editor must focus the final interactive
    mount, not its loading placeholder.** `PlatformComposerEditor`
    deliberately mounts a disabled CM6 instance while the Vim chunk is
    loading, then replaces it with a Vim-enabled instance. A one-frame
    `focusEnd()` from Queue/Draft edit entry can land on the placeholder;
    the replacement then loses keyboard ownership, so `i` never reaches
    the Normal-mode command sink. Carry the end-selection intent through
    the runtime transition and autofocus only the final interactive CM6
    mount. Apply the same contract to inline and fullscreen pending edits;
    Draft and Queue share `PendingRow`. Keep touch entry on its synchronous
    native-textarea focus path so UIKit paste, selection, keyboard, and IME
    ownership remain unchanged. The outer Desktop command capture must also
    treat a Vim sink as an editor when its containing `prompt.draft` or
    `prompt.queued` region is focused; otherwise its list-level `i` edit command
    prevents the already-open editor from receiving Vim's Insert command. Do
    not focus the command sink until its codemirror-vim compatibility handle is
    connected, and retry that connection on the first keydown; an early focused
    sink with a null handle silently drops the user's first Normal command. The
    outer capture must consult the containing region's committed
    `data-desktop-focused` marker rather than an effect closure that can lag the
    edit-opening render and misroute that same first key to the list keymap.

74. **Pending edit surfaces must preserve unplaced attachment previews, and a
    completed mobile delivery must end its keyboard presentation immediately.**
    Older or cross-client Draft/Queue records can contain an image attachment
    without a matching `cowboy-att:` token. The parked card correctly shows that
    image in its tray. Seed pending edits through `promoteUnplacedImageTokens`
    so legacy images regain a deterministic inline position, and derive both
    compact and fullscreen fallback trays from
    `attachmentTrayForSurface(editAttachments, draft)`; filtering the edit tray
    to non-images makes the attachment appear to vanish. Separately,
    WKWebView can keep reporting its keyboard-resized viewport after delivery.
    Latch `mobileKeyboardDismissed` before blurring on authoritative success and
    do not clear that latch merely because the stale `keyboardOpen` signal stays
    true. Only a fresh pointer interaction in the editor may reopen the focused
    surface. Queue and Draft row taps are that same fresh interaction: clear the
    latch in `beginEdit` and on the pending editor-area pointer, and arm the
    focus-transfer window so a transient visualViewport close during the handoff
    cannot freeze the row as "N Drafts Editing" after the keyboard is already
    up. The pending card is a flex column that pins
    `MobileComposerAccessoryDock` and caps the editor/preview area so a tall
    image cannot clip the two-track chrome. Apply the focused surface whenever
    the measured keyboard is open, not only under `:focus-within`. This keeps a
    sent, empty composer content-tight without changing UIKit focus, paste,
    selection, or IME ownership.

75. **PWA `--kb-inset` must follow the painted page, not `window.innerHeight`.**
    Native shell keeps `--kb-inset` at 0 because WKWebView already resizes.
    Browser/PWA still need a visualViewport correction when iOS refuses to
    shrink the layout, but `innerHeight - visualViewport.height` is the wrong
    overlap: Safari often leaves `innerHeight` on the pre-keyboard viewport
    after `interactive-widget=resizes-content` or its compact URL bar has
    already shortened `html`/`#root`. Padding that stale delta paints a
    lavender band above chrome that is already outside the webview (WeType
    and the compact `cowboy.stormbird.xyz` bar make it intermittent). Measure
    `min(innerHeight, documentElement.clientHeight, #root.clientHeight)`
    against `visualViewport.height`. Subtract *clamped* `offsetTop` so a
    pan that keeps the focused field on screen is not counted as cover
    (a rubber-band spike is clamped and cannot inflate the inset).
    **Safari tabs after v1435 still showed the band:** resizes-content
    already parked the painted box above the keyboard; the remaining
    50–110px is iOS 26's compact URL pill sitting *outside* that box,
    usually with `offsetTop = 0`. Treat remainders at or below
    `keyboardOpenMinOverlapPx` (120, the same floor as "is the keyboard
    open") as chrome, not cover. Real keyboards are ~260–340px and still
    pad. PWA has no pill (remainder ≈ 0). Keyboard-open detection may
    keep using `innerHeight` so a stale-large window still counts as an
    open keyboard; only the padding uses the painted box.

76. **A parked/sent `cowboy-att` image must not become a blank rounded box.**
    Three things stacked into the reported "这里为什么不展示图片了呢":
    mdlive hides inactive `![alt](url)` for upstream `image-blocks` that Cowboy
    never vendors, so a read-only MessagePreview (every line inactive) erased
    the inline widget; `attachmentTrayForSurface` then hid the same token id
    from the tray, leaving only the surrounding text; and a reconstructed
    history/draft block whose `data` was externalized to `/api/artifacts/…`
    still minted `data:image/…;base64,` (empty), which `<img>` paints as a
    white hole. Leave `cowboy-att:` Image nodes to `inlineImages.ts`, treat
    empty/HEIC/`cowboy-att:` URLs as unloadable (tray fallback, no dead
    `<img>`), rebuild previews from `url` when `data` is gone, and strip
    placement tokens from transcript Markdown text.

77. **PWA cannot strip the iOS form accessory (∧ ∨ ✓); lift content instead.**
    The native shell removes `inputAccessoryView`. Safari / installed PWA
    cannot. `interactive-widget=resizes-content` often shrinks the painted
    page to the *keyboard* top, so `--kb-inset` becomes 0 (pitfall #75).
    A content-height New Session sheet then parked Title against that
    bar. **Fix:** make New Session a mobile `cover` like Session info
    so Title lives at the top. Do **not** fold 44px into `--kb-inset`:
    after resizes-content the accessory sits *below* the visual
    viewport, and that extra pad becomes an empty band between the
    composer and `∧ ∨ ✓`. Native shell stays at `--kb-inset: 0`. Do
    not try to hide the accessory from the web page.
    **Safari tabs:** html stays tall, `100dvh` cover is taller than the
    visual viewport, and `offsetTop` pans the window onto the cover's
    empty body + Cancel/Create (Title gone). Pin covers to
    `--vv-offset` / `--vv-height`. Leave compact Obsidian cards on
    their own bottom docking.

78. **Dock Paste is a clipboard port, not a native-only bridge.** The
    native shell can probe UIPasteboard and read image bytes without a
    browser permission prompt. Safari / PWA cannot: the dedicated Paste
    button used to stay disabled, and keyboard-shelf photos ("拷贝的图片")
    often arrive as an empty DataTransfer on a `<textarea>`. One
    `ClipboardPort` picks `native` vs `web`; the dock never branches on
    `isNativeShell()`. Web offers Paste when the Clipboard API exists
    and reads on the tap. The textarea still owns the OS paste event
    and falls back to the web port only when the DataTransfer has no
    files. Do not poll the web pasteboard.

79. **Expand → collapse must not forget the software keyboard is up.**
    Fullscreen compose remounts the compact editor. In that focus gap
    `layoutHeight === visualHeight` (PWA already shrank to the keyboard)
    and `inferKeyboardOpen` is false. Learning that shrunken height as
    the rest baseline then leaves `data-mobile-keyboard-open` off after
    focus returns: session nav reappears, the format dock stays collapsed,
    and the compact card fills the leftover canvas. Keep the tall
    baseline unless the visible height is within 80px of it; ignore
    visualViewport "close" during `beginMobileEditorFocusTransfer`;
    reseed the baseline on `orientationchange`.

80. **Native iOS Pinyin candidate taps must commit; do not abort composition.**
    Symptom: tap a word in the system candidate bar (`的` / `调整` / …);
    the word never appears and marked text dies (`tue w` leftover). WeChat
    IME is fine; Obsidian is fine. This is **not** pitfall #69 (Return
    after image). Native Pinyin confirms a candidate with
    `insertReplacementText` / `insertFromComposition` while composition
    is still open. WeChat often commits later as `insertText`.
    Cowboy was aborting that transaction in two Obsidian-divergent
    ways: (1) `mobileEmptyLineCaret` dispatched a CM6 effect on
    `compositionstart` / `insertCompositionText` / `compositionend` —
    iOS Safari first *clears* marked text then reinserts the finished
    word, and a mid-composition `view.dispatch` makes the reinsert
    miss; (2) a live-preview promotion could remount textarea ↔ CM6
    during the same composition. **Fix:** never dispatch or
    `preventDefault` IME beforeinput types; hold the committed editor
    host until `compositionend`. Aligns with Obsidian (same CM6 stack,
    no host swap, no composition-time transactions). Do **not** add
    another Return/`<br>` intercept; #69 stays open.

81. **Obsidian compact sheets are opaque cards with a hairline, not frost.**
    Clear/Symbols used a translucent `backdrop-filter` slab with no
    border. That read as a hollow frame of dimmed page around the card
    (Obsidian has a 1px stroke and solid fill; the 8px edge gap is only
    an optical inset). Keep `background.paper`, a 1px hairline, and
    left/right Cancel/Confirm. Do not restore frost on `ObsidianSheet`.
    Separately, a focused Mobile **draft** dock must keep Send on the
    primary right slot (same place as the main composer Send). Expand
    stays in the left cluster so the empty middle of the top track is
    not a missing action.
