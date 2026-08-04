import { assertEquals } from "jsr:@std/assert";

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

Deno.test("native textarea fills the complete promoted mobile canvas", () => {
  assertEquals(textareaSource.includes("data-mobile-native-editor"), true);
  assertEquals(
    composerSource.includes(
      '"&:has([data-mobile-editor-area]:focus-within) [data-mobile-native-editor] textarea"',
    ),
    true,
  );
  assertEquals(composerSource.includes('height: "100% !important"'), true);
});
