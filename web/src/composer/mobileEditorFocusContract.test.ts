import { assertEquals } from "jsr:@std/assert";
import { didMobileSoftwareKeyboardClose } from "./mobileComposerFocus.ts";

const composerSource = await Deno.readTextFile(
  new URL("../Composer.tsx", import.meta.url),
);
const textareaSource = await Deno.readTextFile(
  new URL("../ComposerTextarea.tsx", import.meta.url),
);

Deno.test("mobile composer promotion is owned by the real editor focus region", () => {
  assertEquals(
    composerSource.includes(
      '"&:has([data-mobile-editor-area]:focus-within)"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes('"&:focus-within [data-mobile-editor-area]"'),
    false,
  );
});

Deno.test("native textarea owns a content-sized mobile canvas", () => {
  assertEquals(textareaSource.includes("data-mobile-native-editor"), true);
  assertEquals(
    composerSource.includes(
      '"&:has([data-mobile-editor-area]:focus-within) [data-mobile-native-editor] textarea"',
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

Deno.test("mobile composer does not reserve an empty strip above its first child", () => {
  assertEquals(composerSource.includes("pt: desktop ? 1 : 0"), true);
  assertEquals(
    composerSource.includes(
      'pt: desktop ? 1 : "var(--mobile-composer-stack-gap)"',
    ),
    false,
  );
  assertEquals(
    composerSource.includes(
      'rowGap: desktop ? 0 : "var(--mobile-composer-stack-gap)"',
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
