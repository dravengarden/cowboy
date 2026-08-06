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

Deno.test("mobile composer promotion requires a visible keyboard and real editor focus", () => {
  assertEquals(
    composerSource.includes(
      '"&[data-mobile-keyboard-open=\'true\']:has([data-mobile-editor-area]:focus-within)"',
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
    composerSource.includes("mobilePendingEditing ? \"56vh\" : \"40vh\""),
    false,
  );
  assertEquals(
    composerSource.includes(
      "mobilePendingKeyboardEditing ? \"none\" : \"40vh\"",
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
  assertEquals(textareaSource.includes("maxRows={expanded ? 30 : 10}"), true);
});

Deno.test("mobile pending edit exits when a third-party IME never reports an open frame", () => {
  assertEquals(
    composerSource.includes(
      "setMobileEditKeyboardSettledClosed(true);\n          finishMobileEditRef.current();\n        },\n        700,",
    ),
    true,
  );
  assertEquals(
    composerSource.includes("if (!mobileEditSawKeyboardRef.current) return undefined"),
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
  const actionRowSource = composerSource.slice(actionRowStart, actionRowStart + 420);

  assertEquals(
    appSource.includes(
      'maxHeight: "100%",\n                                    display: "flex",\n                                    flexDirection: "column"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      '"&[data-mobile-keyboard-open=\'true\']:has([data-mobile-editor-area]:focus-within) [data-mobile-native-editor] .MuiInputBase-input"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes('overflowY: "auto !important"'),
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
    composerSource.includes(
      "> [data-composer-stack-slot]:not([data-composer-stack-slot='primary'])\": {\n            display: \"none\"",
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
    appSource.includes("data-mobile-composer-shell-material=\"true\""),
    true,
  );
  assertEquals(
    appSource.includes(
      "[data-mobile-composer-shell-material='true']\": {\n                        opacity: \"0 !important\"",
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
    3,
  );
});

Deno.test("mobile keyboard dismissal belongs to the fixed utility rail", () => {
  const utilityStart = composerSource.indexOf(
    "data-mobile-composer-utility-rail",
  );
  const utilityEnd = composerSource.indexOf(
    '<Tooltip title={expanded ? "Collapse editor"',
    utilityStart,
  );
  const actionStart = composerSource.indexOf("data-mobile-action-row");
  const actionEnd = composerSource.indexOf(
    "<ComposerToolbarSettings",
    actionStart,
  );
  const utilityRail = composerSource.slice(utilityStart, utilityEnd);
  const actionRow = composerSource.slice(actionStart, actionEnd);

  assertEquals(utilityStart >= 0 && utilityEnd > utilityStart, true);
  assertEquals(actionStart >= 0 && actionEnd > actionStart, true);
  assertEquals(utilityRail.includes("data-mobile-keyboard-hide"), true);
  assertEquals(actionRow.includes("data-mobile-keyboard-hide"), false);
  assertEquals(actionRow.includes("data-mobile-composer-clear"), true);
  assertEquals(actionRow.includes("disabled={!clearable}"), true);
  assertEquals(actionRow.includes("data-mobile-scrollable-actions"), true);
  assertEquals(actionRow.includes('justifyContent: "flex-start"'), true);
  assertEquals(actionRow.includes("WebkitMaskImage: mobileActionEdges"), true);
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
  assertEquals(composerSource.includes("setMobileInputResetBlocked(true)"), true);
  assertEquals(composerSource.includes("disableRestoreFocus={"), true);
  assertEquals(composerSource.includes("data-pending-edit-target"), true);
});

Deno.test("a context reset gate can only reopen from a new input interaction", () => {
  assertEquals(shouldPresentMobileKeyboardSurface(false, false), false);
  assertEquals(shouldPresentMobileKeyboardSurface(true, false), true);
  assertEquals(shouldPresentMobileKeyboardSurface(false, true), false);
  assertEquals(shouldPresentMobileKeyboardSurface(true, true), false);
});
