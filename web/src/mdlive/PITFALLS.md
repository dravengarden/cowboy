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
    transition, which paints a second adjacent caret on an empty line. The theme
    hides only that Vim cursor layer while `.cm-editor.cm-focused`; command-sink
    Normal/Visual states are not `.cm-focused`, so their block cursor and custom
    Visual decorations remain intact. Never solve this by hiding the native
    caret or adding `drawSelection()`—both break IME ownership.

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
    only; Mobile receives neither the focus reset nor the macro UI. Prompt's
    bare `P/O/D/E` region shortcuts must also yield while the command sink owns
    focus, or they shadow native Vim paste, macro, delete, and motion commands.
    The sole narrow exception is `P` on a completely empty main composer in
    Normal mode: there is no document-relative paste target, so the visible Plan
    hint may move focus to Plan. As soon as text or an attachment exists, `p/P`
    belongs to Vim again and the Plan hint is hidden.
    The exception must follow the actual focused `[data-vim-command-sink]`, not
    React's asynchronously mirrored Vim-mode store. With a CJK input source,
    that sink may receive `event.key=Process` / keyCode 229 even though it cannot
    compose; match the physical `KeyP` there. Never enable physical bare-key
    matching on `.cm-content`, where it would steal real Insert-mode text.

17. **Queue/Draft disclosure motion must preserve mounted editor state.** These
    panels use the same MUI `Collapse` motion as Plan. Keep its default mounted
    children behavior so an inline queued/draft editor, attachment preview, and
    sortable registration are not recreated by every fold. Composer height is
    observed by the single persistent ResizeObserver in `App.tsx`; do not add a
    panel-local observer or per-frame React state. The Collapse height layer uses
    `will-change: height`, and its wrapper inner uses `contain: layout paint` so a
    large attachment preview is laid out once instead of producing a long paint
    stall during every reveal frame. Re-verify touch scrolling and inline-editor
    focus after changing this animation.

18. **Fullscreen row edits expose one completion action.** A queued/draft edit
    is live-saved, so both Collapse and Done previously committed the same state
    and closed the overlay. Hide Collapse for row edits and retain the top-right
    Done check as the single explicit completion action. The main new-message
    composer still needs both controls because Collapse preserves the draft while
    Send submits it. Keep this distinction explicit through `showCollapse`; do
    not infer it from labels or alter editor, focus, or IME behavior.

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

20. **Fullscreen row-edit chrome is one non-selectable keyboard dock.** The
    formatting toolbar, edit-mode Save action, and toolbar settings share one
    three-column row: horizontally scrolling commands on the left, an exact-centre
    54px liquid-glass check action, and settings on the right. The dock itself is
    transparent; glass belongs to the primary action, not a keyboard-like second
    surface spanning the screen. Do not restore the wide Save pill, opaque second
    Save row, or a detached dismissal pill: on iOS each wastes the
    keyboard-adjacent area and the detached label can acquire native text-selection
    handles. The dock and its controls use `user-select: none`; this applies only
    to chrome, never the CM6 canvas, so native caret, selection, long-press Paste,
    and IME behavior remain unchanged. The top-right ignore-modifications action
    and confirmation dialog stay separate from Save.

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
- [ ] `a/A/o/O/s/S/C/R` and `c{motion}` enter Insert with a visible native caret;
      for `o/O`, the Selection anchor is the new active `.cm-line`, not `.cm-content`.
- [ ] `.cm-content` stays the identical DOM node and remains `contenteditable=true`.
- [ ] Composition text is accepted with a visible, focused caret after the switch.
- [ ] Escape returns focus to the command sink and the status line shows `IME SAFE`.
- [ ] With an empty Normal-mode composer and a visible Plan, physical `p` focuses
      and expands Plan even while a CJK input source reports `Process` / 229;
      after text or an attachment exists, `p/P` remains native Vim paste.
- [ ] At a mobile viewport, the Desktop Vim/IME chunk is not requested and Mobile
      editor behavior is unchanged.
