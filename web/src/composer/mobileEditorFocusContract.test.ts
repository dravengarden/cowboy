import { assertEquals } from "jsr:@std/assert";
import { didMobileSoftwareKeyboardClose } from "./mobileComposerFocus.ts";

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
      "const mobilePendingKeyboardEditing = mobilePendingEditing && keyboardOpen;",
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
  assertEquals(composerSource.includes("minHeight: 132"), false);
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
    composerSource.includes("() => finishMobileEditRef.current(),\n        700,"),
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
      "const keyboardBoundEditing = editing && (\n    !touchInput || keyboardOpen || !mobileEditSawKeyboardRef.current",
    ),
    true,
  );
  assertEquals(composerSource.includes("if (keyboardBoundEditing) {"), true);
});

Deno.test("mobile composer preserves its stack gap below the transcript boundary", () => {
  assertEquals(
    composerSource.includes(
      'pt: desktop ? 1 : "var(--mobile-composer-stack-gap)"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      'rowGap: desktop ? 0 : "var(--mobile-composer-stack-gap)"',
    ),
    true,
  );
});

Deno.test("mobile keyboard focus presents one floating composer surface", () => {
  assertEquals(
    composerSource.includes("data-mobile-primary-composer"),
    true,
  );
  assertEquals(
    composerSource.includes(
      "> [data-mobile-input-context]\": {\n            display: \"none\"",
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
});

Deno.test("mobile keyboard dismissal belongs to the delivery row, not the utility rail", () => {
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
  assertEquals(utilityRail.includes("data-mobile-keyboard-hide"), false);
  assertEquals(actionRow.includes("data-mobile-keyboard-hide"), true);
  assertEquals(
    actionRow.indexOf('<Tooltip title="Force push">') <
      actionRow.indexOf("data-mobile-keyboard-hide"),
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
