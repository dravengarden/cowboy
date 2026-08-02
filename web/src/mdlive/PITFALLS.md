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
    only; Mobile receives neither the focus reset nor the macro UI. Prompt
    region shortcuts use modified global chords (`Mod+P/I/Y/D`) so bare Vim
    paste, macro, delete, and motion commands always remain editor-owned.
    The same CJK input source can mark any physical Normal-mode keydown on the
    non-editable sink as `isComposing`/229 even though no marked-text transaction
    exists there. Do not discard those events: route the complete letter,
    punctuation, motion, operator, count, and special-key map through
    `event.code` and `vimCommandKey`. Only the runtime's actual
    `compositionstart`/`compositionend` lifecycle (and `EditorView.composing`)
    may suspend sink commands. Workspace/list/config shortcuts follow the same
    physical-key rule outside editable targets; real composition inside
    `.cm-content`, input, or textarea remains exclusively owned by the IME.

17. **Queue/Draft disclosure motion must preserve mounted editor state.** These
    panels use the same MUI `Collapse` motion as Plan. Keep its default mounted
    children behavior so an inline queued/draft editor, attachment preview, and
    sortable registration are not recreated by every fold. Composer height is
    observed by the single persistent ResizeObserver in `App.tsx`; do not add a
    panel-local observer or per-frame React state. The Collapse height layer uses
    `will-change: height`, and its wrapper inner uses `contain: layout paint` so a
    large attachment preview is laid out once instead of producing a long paint
    stall during every reveal frame. Re-verify touch scrolling and inline-editor
    focus after changing this animation. Mobile row edits begin in this compact
    card instead of navigating directly to fullscreen. The card uses the shared
    two-track accessory primitive and exposes one explicit expand action; opening
    fullscreen transfers the same draft and attachment state without changing
    save semantics.

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

20. **Fullscreen row-edit chrome is one non-selectable two-track keyboard dock.**
    The upper track owns only horizontally scrolling formatting commands. The
    lower track owns completion and editor-level actions, with Settings fixed in
    its final 44pt slot. This keeps Done stable without mixing document formatting
    and message lifecycle in one crowded rail. Do not restore the wide Save pill,
    opaque detached rows, or a dismissal pill: on iOS each wastes the
    keyboard-adjacent area and detached labels can acquire native text-selection
    handles. The dock and its controls use `user-select: none`; this applies only
    to chrome, never the CM6 canvas, so native caret, selection, long-press Paste,
    and IME behavior remain unchanged. The top-right ignore-modifications action
    and confirmation dialog stay separate from Done.

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
    Keep one `KeyboardHide` icon in the existing right-hand dock utilities. It
    blurs only the currently active element; it must not reconfigure CM6, mutate
    the document, collapse the fullscreen surface, or add a detached dismissal
    pill. Tapping the editor restores native focus normally. This fills the
    otherwise unused balanced-dock slot while preserving the exact-centre primary
    action and the native caret/IME ownership rules above.

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
    settle after the old 700ms final sample. **The fallback must derive overlap
    from the keyboard frame's own height, never `parentBottom - frame.minY`:** the
    latter includes the stale coordinate offset and over-shrinks the WKWebView,
    recreating a large blank region above an iPad split keyboard. A frame that
    genuinely reaches the current parent bottom still uses the normal
    `parentBottom - frame.minY` intersection. Ignore face-up/face-down device
    motion so ordinary handling cannot reset a visible keyboard. Verify the exact
    cold sequence: keyboard hidden → rotate → first focus, in both directions,
    plus split and floating keyboard regressions.

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
    keyboard, attachment, and Send/Done lifecycle actions; overflow then made the
    stable Settings area look like a selected column. `MobileComposerAccessoryDock`
    now owns one 96px material with two semantic tracks: keyboard/attachment and
    contextual Send/Done remain stable above, while formatting scrolls alone on
    the track nearest the keyboard. Fullscreen editors render that material as
    the same inset, rounded panel used by the compact composer, not as two
    edge-to-edge system bars. Both surfaces use the same 8px Mobile composer
    gutter; do not introduce a separate fullscreen inset. Its only internal
    separator is the quiet horizontal boundary between semantic tracks. Settings
    is an ordinary trailing 44pt
    formatting action, never a selected-looking rail, gradient, or separately
    divided region. Main fullscreen compose also moves Collapse into the upper
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
    contrasting block. Focus may increase the editor canvas, but the card radius
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
    MUI cannot take focus from the editor before click; click semantics and
    keyboard accessibility remain intact. Fullscreen Collapse is the one action
    that replaces the focused editor: synchronously mount the compact editor and
    transfer focus within the originating gesture. Never replace either rule
    with a timer, which runs after UIKit has ended the keyboard transaction.

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
    next quiet page-head group. A failure replaces the skeleton with a
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
    `ComposerToolbarSettings` therefore blurs only an active native textarea or
    CM contenteditable in a layout effect when opening. Do not move that blur to
    the shared accessory button primitive: ordinary actions must continue to
    preserve keyboard focus.

38. **A full-height touch Composer without inline images stays a native
    textarea.** Resolving the complete CM6 height chain makes the blank canvas a
    real `contenteditable` hit target, but physical iPhone testing shows that
    WebKit still anchors its edit menu unreliably when a long press is far from
    the nearest real text line. The compact editor did not reproduce the bug
    because UIKit owned a native textarea. Fullscreen and expanded touch editors
    now use that same native control for token-free text, including literal
    Markdown toolbar transformations and selection reporting. An inline-image
    token still promotes the document synchronously to CM6 so its widget remains
    in flow. Keep the live native value separate from CM6's frozen mount seed;
    never emulate this with a transparent overlay, synthetic pointer selection,
    or delayed refocus.

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
- [ ] Typing/composition keeps the same `.cm-content` node and emits no callback-
      identity `StateEffect.reconfigure` after the document transaction.
- [ ] Escape returns focus to the command sink and the status line shows `IME SAFE`.
- [ ] `Mod+P` focuses and expands a visible Plan from any Agent region;
      bare `p/P` remains native Vim paste in every composer state.
- [ ] At a mobile viewport, the Desktop Vim/IME chunk is not requested and Mobile
      editor behavior is unchanged.
