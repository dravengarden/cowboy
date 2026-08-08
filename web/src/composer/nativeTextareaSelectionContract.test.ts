import { assertEquals } from "jsr:@std/assert";

const textareaSource = await Deno.readTextFile(
  new URL("../ComposerTextarea.tsx", import.meta.url),
);

Deno.test("native textarea keeps React rerenders from replacing iOS selection", () => {
  assertEquals(textareaSource.includes("defaultValue={value}"), true);
  assertEquals(textareaSource.includes("        value={value}\n"), false);
  assertEquals(
    textareaSource.includes("lastNativeValueRef.current === ta.value"),
    true,
  );
  assertEquals(
    textareaSource.includes("mapNativeSelectionThroughValueChange"),
    true,
  );
});
