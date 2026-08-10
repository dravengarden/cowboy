import { assertEquals } from "jsr:@std/assert";

const textareaSource = await Deno.readTextFile(
  new URL("../ComposerTextarea.tsx", import.meta.url),
);
const editorSource = await Deno.readTextFile(
  new URL("../ComposerEditor.tsx", import.meta.url),
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

Deno.test("native textarea and CM6 expose the same logical selection handoff", () => {
  assertEquals(textareaSource.includes("getSelection: ()"), true);
  assertEquals(
    textareaSource.includes('selectionDirection === "backward"'),
    true,
  );
  assertEquals(textareaSource.includes("focusSelection: (selection"), true);
  assertEquals(editorSource.includes("getSelection: ()"), true);
  assertEquals(editorSource.includes("focusSelection: (selection"), true);
  assertEquals(editorSource.includes("scrollIntoView: true"), true);
  assertEquals(textareaSource.includes("insertText: ("), true);
  assertEquals(editorSource.includes("insertText: ("), true);
  assertEquals(textareaSource.includes("replaceNativeSelection"), true);
  assertEquals(textareaSource.includes("lastSelectionRef"), true);
  assertEquals(textareaSource.includes("rememberedSelection(ta)"), true);
});
