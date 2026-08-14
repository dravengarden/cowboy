import { assertEquals } from "jsr:@std/assert";
import {
  didMobileSoftwareKeyboardClose,
  mobilePendingKeyboardCloseSettleMs,
  shouldPresentMobileKeyboardSurface,
} from "./mobileComposerFocus.ts";
import { mobileComposerStackGap } from "../mobileComposerPrimitives.ts";

const composerSource = await Deno.readTextFile(
  new URL("../Composer.tsx", import.meta.url),
);
const textareaSource = await Deno.readTextFile(
  new URL("../ComposerTextarea.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("../App.tsx", import.meta.url),
);
const fullscreenComposerSource = await Deno.readTextFile(
  new URL("../FullscreenComposer.tsx", import.meta.url),
);
const platformEditorSource = await Deno.readTextFile(
  new URL("./PlatformComposerEditor.tsx", import.meta.url),
);
const accessoryDockSource = await Deno.readTextFile(
  new URL("../MobileComposerAccessoryDock.tsx", import.meta.url),
);
const formatActionsSource = await Deno.readTextFile(
  new URL("../MobileComposerFormatActions.tsx", import.meta.url),
);
const settingsPointerDown = appSource.lastIndexOf(
  "settingsTap.onPointerDown",
);
const mobileSettingsStart = appSource.lastIndexOf(
  "<IconButton",
  settingsPointerDown,
);
const mobileSettingsEnd = appSource.indexOf(
  "</IconButton>",
  settingsPointerDown,
);
const mobileSettingsSource = appSource.slice(
  mobileSettingsStart,
  mobileSettingsEnd,
);

Deno.test("mobile composer promotion requires a visible keyboard and real editor focus", () => {
  assertEquals(
    composerSource.includes(
      "\"&[data-mobile-keyboard-open='true']:has([data-mobile-editor-area]:focus-within)\"",
    ),
    true,
  );
  assertEquals(composerSource.includes("data-mobile-keyboard-open={"), true);
  assertEquals(
    composerSource.includes(
      '"&:has([data-mobile-editor-area]:focus-within)"',
    ),
    false,
  );
});

Deno.test("mobile keyboard keeps the writing material opaque across transient WebKit focus loss", () => {
  assertEquals(
    composerSource.includes(
      "\"&[data-mobile-keyboard-open='true']\": {\n              ...mobileFocusedComposerSurfaceSx",
    ),
    true,
  );
});

Deno.test("native fullscreen keyboard spacing follows measured keyboard state", () => {
  assertEquals(
    fullscreenComposerSource.includes(
      "const keyboardOpen = useKeyboardOpen();",
    ),
    true,
  );
  assertEquals(
    fullscreenComposerSource.includes(
      'data-mobile-keyboard-open={keyboardOpen ? "true" : undefined}',
    ),
    true,
  );
  assertEquals(
    fullscreenComposerSource.includes(
      "nativeShell\n          ? keyboardOpen\n            ? `${mobileComposerKeyboardGap}px`",
    ),
    true,
  );
  assertEquals(
    fullscreenComposerSource.includes('"&:focus-within": { pb:'),
    false,
  );
});

Deno.test("mobile session navigation stays tappable after keyboard dismissal", () => {
  assertEquals(
    appSource.includes(
      "[data-mobile-focus-composer='true'][data-mobile-keyboard-open='true']:focus-within) [data-mobile-session-nav='true']",
    ),
    true,
  );
  assertEquals(
    appSource.includes(
      "[data-mobile-focus-composer='true']:focus-within) [data-mobile-session-nav='true']",
    ),
    false,
  );
});

Deno.test("mobile settings commits touch taps when Safari drops synthetic click", () => {
  assertEquals(
    mobileSettingsStart >= 0 && mobileSettingsEnd > mobileSettingsStart,
    true,
  );
  assertEquals(
    mobileSettingsSource.includes("settingsTap.onPointerUp"),
    true,
  );
  assertEquals(
    mobileSettingsSource.includes("settingsTap.onPointerCancel"),
    true,
  );
  assertEquals(mobileSettingsSource.includes("settingsTap.onClick"), true);
});

Deno.test("mobile settings does not retain synthetic touch hover paint", () => {
  assertEquals(
    mobileSettingsSource.includes(
      'event.currentTarget.dataset.touchActivated = "true"',
    ),
    true,
  );
  assertEquals(
    mobileSettingsSource.includes(
      "&[data-touch-activated='true']:hover, &[data-touch-activated='true'].Mui-focusVisible",
    ),
    true,
  );
  assertEquals(
    mobileSettingsSource.match(
      /delete event\.currentTarget\.dataset\.touchActivated/g,
    )?.length,
    3,
  );
  assertEquals(
    mobileSettingsSource.includes(
      "&[data-touch-activated='true']:active",
    ),
    true,
  );
});

Deno.test("Machine npm updates survive a swallowed Safari click", () => {
  const updateButtonStart = appSource.indexOf(
    "function MachineNpmUpdateButton",
  );
  const updateButtonEnd = appSource.indexOf(
    "function MachinesContent",
    updateButtonStart,
  );
  const updateButton = appSource.slice(updateButtonStart, updateButtonEnd);

  assertEquals(
    updateButtonStart >= 0 && updateButtonEnd > updateButtonStart,
    true,
  );
  assertEquals(
    updateButton.includes("useReliableTouchTap<HTMLButtonElement>(onUpdate)"),
    true,
  );
  assertEquals(updateButton.includes("{...updateTap}"), true);
});

Deno.test("mobile recommended presets use the Cowboy control radius", () => {
  const presetsStart = composerSource.indexOf(
    "{recommendedPresets.map((preset) => {",
  );
  const presetsEnd = composerSource.indexOf(
    "<ButtonBase\n                        aria-expanded",
    presetsStart,
  );
  const presets = composerSource.slice(presetsStart, presetsEnd);

  assertEquals(presetsStart >= 0 && presetsEnd > presetsStart, true);
  assertEquals(presets.includes("borderRadius: 1,"), true);
  assertEquals(presets.includes("borderRadius: 2,"), false);
});

Deno.test("mobile column and pending ownership cannot fill the viewport without a keyboard", () => {
  assertEquals(
    composerSource.includes(
      "const mobilePendingKeyboardEditing = mobilePendingEditing &&\n    mobileKeyboardPresentationOpen;",
    ),
    true,
  );
  assertEquals(
    composerSource.includes("...(desktop && column && {"),
    true,
  );
  assertEquals(composerSource.includes("...(column && { flex: 1 })"), false);
  assertEquals(composerSource.includes("fill={desktop && column}"), true);
  assertEquals(
    composerSource.includes(
      'maxHeight: mobilePendingKeyboardEditing ? "none" : "40vh"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      'overflowY: mobilePendingKeyboardEditing ? "hidden" : "auto"',
    ),
    true,
  );
});

Deno.test("Plan plus Queue or Draft editing cannot promote mobile geometry after keyboard dismissal", () => {
  assertEquals(
    composerSource.includes("...(desktop && column && {"),
    true,
  );
  assertEquals(
    composerSource.includes('mobilePendingEditing ? "56vh" : "40vh"'),
    false,
  );
  assertEquals(
    composerSource.includes(
      'mobilePendingKeyboardEditing ? "none" : "40vh"',
    ),
    true,
  );
});

Deno.test("native textarea owns a content-sized mobile canvas", () => {
  assertEquals(textareaSource.includes("data-mobile-native-editor"), true);
  assertEquals(
    composerSource.includes(
      "[data-mobile-native-editor] textarea",
    ),
    false,
  );
  assertEquals(
    composerSource.includes(
      "minHeight: MOBILE_COMPOSER_IDLE_EDITOR_MIN_H",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "minHeight: MOBILE_COMPOSER_INPUT_EDITOR_MIN_H",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "data-mobile-editor-area] > *, &:has([data-mobile-editor-area]:focus-within)",
    ),
    false,
  );
  assertEquals(composerSource.includes('"& > *": { flex: 1 }'), false);
  assertEquals(textareaSource.includes('component="textarea"'), true);
  assertEquals(textareaSource.includes("<TextField"), false);
});

Deno.test("mobile pending edit exits when a third-party IME never reports an open frame", () => {
  assertEquals(
    composerSource.includes(
      "setMobileEditKeyboardSettledClosed(true);\n          finishMobileEditRef.current();\n        },\n        700,",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "if (!mobileEditSawKeyboardRef.current) return undefined",
    ),
    false,
  );
  assertEquals(
    composerSource.includes(
      'display: !desktop && mobilePendingKeyboardEditing ? "none" : "flex"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "const keyboardBoundEditing = editing && (\n    !touchInput || !mobileEditKeyboardSettledClosed",
    ),
    true,
  );
  assertEquals(composerSource.includes("if (keyboardBoundEditing) {"), true);
});

Deno.test("mobile pending editor survives the native long-press keyboard settle window", () => {
  assertEquals(mobilePendingKeyboardCloseSettleMs, 550);
  assertEquals(
    composerSource.includes(
      "setMobileEditKeyboardSettledClosed(true);\n        finishMobileEditRef.current();",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "mobilePendingKeyboardCloseSettleMs,\n    );",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "const frame = globalThis.requestAnimationFrame(() =>\n      finishMobileEditRef.current()",
    ),
    false,
  );
});

Deno.test("fullscreen pending edit distinguishes collapse from keyboard dismissal", () => {
  assertEquals(
    composerSource.includes(
      'submitLabel={touchInput ? "Collapse editor" : "Done editing"}',
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "submitIcon={touchInput ? <CloseFullscreen /> : <Check />}",
    ),
    true,
  );
});

Deno.test("mobile compact and fullscreen handoffs preserve selection with one focus transfer", () => {
  assertEquals(
    composerSource.includes(
      "const selection = editorRef.current?.getSelection();\n                    flushSync(() => setComposeFs(true));",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "editorRef.current?.focusSelection(selection);",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "if (!overlayOpen || touchInput) return undefined;",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "overlayEditorRef.current?.focusSelection(selection);",
    ),
    true,
  );
});

Deno.test("pending-row image paste stages synchronously before encoding", () => {
  const pendingStart = composerSource.indexOf("if (keyboardBoundEditing) {");
  const pendingEnd = composerSource.indexOf(
    "// Secondary actions",
    pendingStart,
  );
  const pending = composerSource.slice(pendingStart, pendingEnd);

  assertEquals(pending.includes("pendingImageAttachment(file)"), true);
  assertEquals(
    pending.includes(
      "activeEditEditor()?.insertImages(pending, options.selection)",
    ),
    true,
  );
  assertEquals(
    pending.includes("addEditFiles(files, { preserveFocus: true })"),
    true,
  );
  assertEquals(pending.includes("!editAttachmentsPending"), true);
});

Deno.test("native image and text paste action is shared by every mobile editor surface", () => {
  assertEquals(
    composerSource.match(/<MobileComposerFormatActions/g)?.length,
    2,
  );
  assertEquals(
    fullscreenComposerSource.includes("<MobileComposerFormatActions"),
    true,
  );
  assertEquals(
    formatActionsSource.includes(
      "capturedSelectionRef.current ??\n      editorRef.current?.getSelection()",
    ),
    true,
  );
  assertEquals(formatActionsSource.includes("pasteTap.onPointerUp"), true);
  assertEquals(
    formatActionsSource.includes("readNativeClipboardImageOutcome"),
    true,
  );
  assertEquals(
    formatActionsSource.includes("return outcome.files"),
    true,
  );
  assertEquals(
    formatActionsSource.includes("readNativeClipboardText"),
    true,
  );
  assertEquals(
    formatActionsSource.includes("insertText(text, selection)"),
    true,
  );
  assertEquals(
    formatActionsSource.includes(
      "event.preventDefault();\n        pasteTap.onClick(event)",
    ),
    true,
  );
  assertEquals(
    formatActionsSource.includes(
      "(reading && availability.hasImages)",
    ),
    true,
  );
  assertEquals(
    textareaSource.includes(
      "attachments.some((attachment) => attachment.pending === true)",
    ),
    true,
  );
  assertEquals(
    /event\.preventDefault\(\);\s+onPointerDown\?\.\(event\);/.test(
      accessoryDockSource,
    ),
    true,
  );
  assertEquals(
    accessoryDockSource.match(
      /onMouseDown=\{\(event\): void => event\.preventDefault\(\)\}/g,
    )?.length,
    3,
  );
  assertEquals(
    textareaSource.includes("ta.focus();\n        writeNativeEdit(ta"),
    true,
  );
});

Deno.test("deleting the last inline image transfers CM6 focus to native text", () => {
  assertEquals(
    platformEditorSource.includes(
      "demotionSelectionRef.current = childEditorRef.current?.getSelection()",
    ),
    true,
  );
  assertEquals(
    platformEditorSource.includes(
      "demotionFocusPendingRef.current = childEditorRef.current?.hasFocus()",
    ),
    true,
  );
  assertEquals(
    platformEditorSource.includes(
      "autoFocus={props.autoFocus || focusDemotedEditor}",
    ),
    true,
  );
  assertEquals(
    platformEditorSource.includes("initialSelection: demotionSelection"),
    true,
  );
  assertEquals(
    textareaSource.includes(
      "const initialSelectionRef = useRef(initialSelection)",
    ),
    true,
  );
  assertEquals(
    textareaSource.includes(
      "ta.setSelectionRange(\n      Math.min(anchor, head)",
    ),
    true,
  );
});

Deno.test("fullscreen keeps view chrome fixed right and send with message actions", () => {
  assertEquals(
    fullscreenComposerSource.includes(
      'title={submitLabel}\n                color="primary"',
    ),
    true,
  );
  assertEquals(
    accessoryDockSource.includes("data-mobile-composer-primary-actions"),
    true,
  );
  assertEquals(
    fullscreenComposerSource.includes(
      'primaryLabel={showCollapse ? "Collapse editor" : submitLabel}',
    ),
    true,
  );
  assertEquals(accessoryDockSource.includes("primaryCompanion"), false);
});

Deno.test("fullscreen primary action survives a swallowed Safari click", () => {
  const primaryStart = accessoryDockSource.indexOf(
    "aria-label={primaryLabel.toLowerCase()}",
  );
  const primaryEnd = accessoryDockSource.indexOf(
    "</IconButton>",
    primaryStart,
  );
  const primaryButton = accessoryDockSource.slice(primaryStart, primaryEnd);

  assertEquals(primaryStart >= 0 && primaryEnd > primaryStart, true);
  assertEquals(
    accessoryDockSource.includes(
      "useReliableTouchTap<HTMLButtonElement>(onPrimary)",
    ),
    true,
  );
  assertEquals(
    primaryButton.includes(
      "event.preventDefault();\n                  primaryTap.onPointerDown(event);",
    ),
    true,
  );
  assertEquals(primaryButton.includes("primaryTap.onPointerMove"), true);
  assertEquals(primaryButton.includes("primaryTap.onPointerUp"), true);
  assertEquals(primaryButton.includes("primaryTap.onPointerCancel"), true);
  assertEquals(primaryButton.includes("primaryTap.onClick"), true);
});

Deno.test("move-draft undo toast clears the iOS status safe area", () => {
  assertEquals(
    composerSource.includes(
      'xs: "calc(env(safe-area-inset-top, 0px) + 8px)"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes('width: { xs: "calc(100% - 16px)", sm: "auto" }'),
    true,
  );
});

Deno.test("every focused mobile editor uses two semantic bars with a fixed keyboard action", () => {
  assertEquals(
    accessoryDockSource.includes("data-mobile-composer-utility-actions"),
    true,
  );
  assertEquals(
    accessoryDockSource.includes("data-mobile-composer-editing-bar"),
    true,
  );
  assertEquals(
    accessoryDockSource.includes("data-mobile-composer-fixed-action"),
    true,
  );
  assertEquals(composerSource.includes("<MobileComposerEditingBar"), true);
  assertEquals(composerSource.includes("data-mobile-keyboard-hide"), false);
  assertEquals(
    fullscreenComposerSource.includes(
      'fixedAction={\n          <MobileComposerAccessoryButton\n            title="Hide keyboard"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes('primaryLabel="Expand editor"'),
    true,
  );
  assertEquals(
    formatActionsSource.includes('title="Customize toolbar"'),
    true,
  );
});

Deno.test("expanded mobile rails align while compact overlay stays undivided", () => {
  assertEquals(
    accessoryDockSource.match(/<MobileComposerFixedActionSlot region=/g)
      ?.length,
    2,
  );
  assertEquals(
    accessoryDockSource.includes("data-mobile-composer-fixed-slot"),
    true,
  );
  assertEquals(
    accessoryDockSource.includes(
      'data-mobile-composer-primary-actions={region === "primary"',
    ),
    true,
  );
  assertEquals(
    accessoryDockSource.includes(
      'data-mobile-composer-fixed-action={region === "editing" ? "" : undefined}',
    ),
    true,
  );
  assertEquals(accessoryDockSource.includes("px: 0.5"), false);
  assertEquals(
    accessoryDockSource.includes("borderLeft: overlay ? 0 : 1"),
    true,
  );
  assertEquals(
    composerSource.includes(
      '<MobileComposerFixedActionSlot region="primary" overlay>',
    ),
    true,
  );
  assertEquals(
    accessoryDockSource.includes(
      'data-mobile-composer-utility-rail={overlay ? "" : undefined}',
    ),
    true,
  );
});

Deno.test("fullscreen delivery closes only after authoritative success", () => {
  assertEquals(
    composerSource.includes(
      "submitWithFeedback(() => setComposeFs(false))",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "dismissAfterMobileDelivery();\n    setComposeFs(false);",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "submitAndNotify();\n            setComposeFs(false);",
    ),
    false,
  );
});

Deno.test("mobile composer keeps one boundary gap across focus transitions", () => {
  assertEquals(mobileComposerStackGap, 4);
  assertEquals(
    composerSource.includes(
      'pt: desktop ? 1 : "var(--mobile-composer-boundary-gap)"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      'rowGap: desktop ? 0 : "var(--mobile-composer-stack-gap)"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      '"--mobile-composer-boundary-gap": `${mobileComposerStackGap}px`',
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      '"--mobile-composer-stack-gap": `${mobileComposerStackGap}px`',
    ),
    true,
  );
  assertEquals(
    composerSource.includes('paddingTop: "4px"'),
    false,
  );
  assertEquals(composerSource.includes('alignItems: "stretch"'), true);
  assertEquals(
    composerSource.includes(
      '"& > *": {\n            width: "100%",\n            minWidth: 0,',
    ),
    true,
  );
});

Deno.test("long touch prompts scroll inside the editor without hiding chrome", () => {
  const formatRowStart = composerSource.indexOf("data-mobile-focus-format-row");
  const actionRowStart = composerSource.indexOf("data-mobile-action-row={");
  const formatRowSource = composerSource.slice(formatRowStart, actionRowStart);
  const actionRowSource = composerSource.slice(
    actionRowStart,
    actionRowStart + 420,
  );

  assertEquals(
    appSource.includes(
      'maxHeight: "100%",\n                                    display: "flex",\n                                    flexDirection: "column"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "\"&[data-mobile-keyboard-open='true']:has([data-mobile-editor-area]:focus-within) [data-mobile-native-editor] [data-mobile-native-textarea='true']\"",
    ),
    true,
  );
  assertEquals(
    composerSource.includes('height: "100% !important"'),
    true,
  );
  assertEquals(
    /flex: "1 1 auto",\s+minHeight: 0,\s+height: "100%"/.test(
      composerSource,
    ),
    true,
  );
  assertEquals(
    textareaSource.includes("nativeTextareaNeedsScroll"),
    true,
  );
  assertEquals(
    textareaSource.includes("overflowY: nativeScrollable"),
    true,
  );
  assertEquals(formatRowStart >= 0, true);
  assertEquals(actionRowStart >= 0, true);
  assertEquals(formatRowSource.includes("flexShrink: 0"), true);
  assertEquals(actionRowSource.includes("flexShrink: 0"), true);
});

Deno.test("mobile keyboard focus presents one floating composer surface", () => {
  assertEquals(
    composerSource.includes("<span data-mobile-composer-clear>"),
    true,
  );
  assertEquals(
    composerSource.indexOf("data-mobile-composer-utility-rail") <
      composerSource.indexOf("<span data-mobile-composer-clear>"),
    true,
  );
  assertEquals(
    composerSource.includes("data-mobile-primary-composer"),
    true,
  );
  assertEquals(
    /> \[data-composer-stack-slot\]:not\(\[data-composer-stack-slot='primary'\]\)":\s*{\s*display: "none"/
      .test(
        composerSource,
      ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "const mobileFloatingEdit = !desktop && editingId !== null && keyboardOpen;",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      'data-mobile-floating-edit={mobileFloatingEdit ? "true" : undefined}',
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "& [data-mobile-pending-row]:not([data-mobile-pending-row-editing='true'])",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      'data-mobile-pending-editor={touchInput ? "true" : undefined}',
    ),
    true,
  );
  assertEquals(
    appSource.includes('data-mobile-composer-shell-material="true"'),
    true,
  );
  assertEquals(
    appSource.includes(
      '[data-mobile-composer-shell-material=\'true\']": {\n                        opacity: "0 !important"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "[data-mobile-primary-composer='true'][data-mobile-keyboard-open='true']:has(",
    ),
    false,
  );
  assertEquals(
    appSource.includes(
      "[data-mobile-pending-editor='true'][data-mobile-keyboard-open='true']:focus-within",
    ),
    true,
  );
  assertEquals(
    composerSource.match(/mobileFocusedComposerSurfaceSx/g)?.length,
    4,
  );
});

Deno.test("mobile pending editing keeps expansion and context delivery actions in the first dock", () => {
  const pendingStart = composerSource.indexOf("if (keyboardBoundEditing) {");
  const pendingEnd = composerSource.indexOf(
    "// Secondary actions",
    pendingStart,
  );
  const pending = composerSource.slice(pendingStart, pendingEnd);

  assertEquals(pendingStart >= 0 && pendingEnd > pendingStart, true);
  assertEquals(
    pending.indexOf('title="Attach file"') <
      pending.indexOf('primaryLabel="Expand editor"'),
    true,
  );
  assertEquals(pending.includes('primaryLabel="Expand editor"'), true);
  assertEquals(pending.includes("onPrimary={expandMobileEdit}"), true);
  assertEquals(pending.includes('title="Send draft"'), true);
  assertEquals(
    /title=\{message\.schedule\s*\?\s*"Reschedule send"\s*:\s*"Schedule send"\}/
      .test(pending),
    true,
  );
  assertEquals(pending.includes('title="Force push"'), true);
  assertEquals(pending.includes("networkAction={sendDraftFromEdit}"), true);
  assertEquals(
    composerSource.includes("const completePendingDelivery = async"),
    true,
  );
  assertEquals(composerSource.includes("if (!persistEdit()) return;"), true);
});

Deno.test("pending Force push confirmation keeps the native editor and anchor mounted", () => {
  const confirmationStart = composerSource.indexOf(
    'const forcePushConfirmation = kind === "queued"',
  );
  const confirmationEnd = composerSource.indexOf(
    "// Mobile Queue/Draft edits are continuously buffered",
    confirmationStart,
  );
  const confirmation = composerSource.slice(confirmationStart, confirmationEnd);

  assertEquals(
    confirmationStart >= 0 && confirmationEnd > confirmationStart,
    true,
  );
  assertEquals(confirmation.includes("<Popper"), true);
  assertEquals(confirmation.includes("<Popover"), false);
  assertEquals(confirmation.includes("<ClickAwayListener"), true);
  assertEquals(confirmation.includes('aria-modal="false"'), true);
  assertEquals(
    confirmation.match(
      /onPointerDown=\{\(event\): void => event\.preventDefault\(\)\}/g,
    )
      ?.length,
    2,
  );
});

Deno.test("mobile keyboard dismissal belongs to the fixed lower editing rail", () => {
  const utilityStart = composerSource.indexOf(
    '<MobileComposerFixedActionSlot region="primary" overlay>',
  );
  const utilityEnd = composerSource.indexOf(
    '<Tooltip title={expanded ? "Collapse editor"',
    utilityStart,
  );
  const actionStart = composerSource.indexOf("data-mobile-action-row");
  const editingStart = composerSource.indexOf("data-mobile-focus-format-row");
  const actionEnd = composerSource.indexOf(
    "<ComposerToolbarSettings",
    actionStart,
  );
  const utilityRail = composerSource.slice(utilityStart, utilityEnd);
  const editingRail = composerSource.slice(editingStart, actionStart);
  const actionRow = composerSource.slice(actionStart, actionEnd);

  assertEquals(utilityStart >= 0 && utilityEnd > utilityStart, true);
  assertEquals(editingStart >= 0 && actionStart > editingStart, true);
  assertEquals(actionStart >= 0 && actionEnd > actionStart, true);
  assertEquals(utilityRail.includes('title="Hide keyboard"'), false);
  assertEquals(editingRail.includes("<MobileComposerEditingBar"), true);
  assertEquals(editingRail.includes('title="Hide keyboard"'), true);
  assertEquals(actionRow.includes('title="Hide keyboard"'), false);
  assertEquals(actionRow.includes("data-mobile-composer-clear"), true);
  assertEquals(actionRow.includes("disabled={!clearable}"), true);
  assertEquals(actionRow.includes("data-mobile-scrollable-actions"), true);
  assertEquals(actionRow.includes('justifyContent: "flex-start"'), true);
  assertEquals(actionRow.includes("WebkitMaskImage:"), true);
  assertEquals(actionRow.includes("mobileActionEdges.left"), true);
  assertEquals(actionRow.includes('columnGap: "clamp(2px, 1vw, 5px)"'), true);
  assertEquals(actionRow.includes('position: "sticky"'), false);
  assertEquals(
    actionRow.indexOf('<Tooltip title="Force push">') <
      actionRow.indexOf('<Tooltip title="定时发送">'),
    true,
  );
  assertEquals(
    actionRow.indexOf('<Tooltip title="定时发送">') <
      actionRow.indexOf("data-mobile-composer-clear"),
    true,
  );
});

Deno.test("mobile delivery taps preserve native editor focus until click", () => {
  const sendStart = composerSource.indexOf('aria-label="send"');
  const sendEnd = composerSource.indexOf("</IconButton>", sendStart);
  const sendButton = composerSource.slice(sendStart, sendEnd);

  assertEquals(sendStart >= 0 && sendEnd > sendStart, true);
  assertEquals(
    /event\.preventDefault\(\);\s+sendTap\.onPointerDown\(event\);/.test(
      sendButton,
    ),
    true,
  );
  assertEquals(
    sendButton.includes("onPointerUp={sendTap.onPointerUp}"),
    true,
  );
  assertEquals(
    composerSource.includes(
      "// Keep the native textarea as iOS's first responder until the tap/hold is",
    ),
    true,
  );
});

Deno.test("Page delivery closes only after the authoritative acknowledgement", () => {
  const submitStart = composerSource.indexOf("const submitWithFeedback");
  const submitEnd = composerSource.indexOf("const sendTap", submitStart);
  const submit = composerSource.slice(submitStart, submitEnd);
  const actionStart = submit.indexOf("submitFeedback.run(() => {");
  const actionEnd = submit.indexOf("if (submitted && succeeded)", actionStart);
  assertEquals(actionStart >= 0 && actionEnd > actionStart, true);
  assertEquals(
    submit.slice(actionStart, actionEnd).includes("onSubmitted?.()"),
    false,
  );
  assertEquals(
    submit.slice(actionEnd).includes(
      "dismissAfterMobileDelivery();\n        // Explore/Page",
    ),
    true,
  );
  assertEquals(submit.slice(actionEnd).includes("onSubmitted?.();"), true);
});

Deno.test("mobile composer releases stale focus only after a visible keyboard closes", () => {
  assertEquals(didMobileSoftwareKeyboardClose(false, false), false);
  assertEquals(didMobileSoftwareKeyboardClose(false, true), false);
  assertEquals(didMobileSoftwareKeyboardClose(true, true), false);
  assertEquals(didMobileSoftwareKeyboardClose(true, false), true);
  assertEquals(
    composerSource.includes("mobileComposerKeyboardWasOpenRef"),
    true,
  );
  assertEquals(
    composerSource.includes("releaseMobileComposerFocus();"),
    true,
  );
});

Deno.test("clearing session context always ends the mobile input interaction", () => {
  assertEquals(
    composerSource.includes(
      "timelineState.contextClearedSeq <= previous.seq",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      'if (touchInput && action.kind === "reset")',
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "requestAnimationFrame(() => releaseMobileComposerFocus())",
    ),
    true,
  );
  assertEquals(
    composerSource.includes("setMobileInputResetBlocked(true)"),
    true,
  );
  assertEquals(composerSource.includes("disableRestoreFocus={"), true);
  assertEquals(composerSource.includes("data-pending-edit-target"), true);
});

Deno.test("a context reset gate can only reopen from a new input interaction", () => {
  assertEquals(shouldPresentMobileKeyboardSurface(false, false), false);
  assertEquals(shouldPresentMobileKeyboardSurface(true, false), true);
  assertEquals(shouldPresentMobileKeyboardSurface(false, true), false);
  assertEquals(shouldPresentMobileKeyboardSurface(true, true), false);
});
