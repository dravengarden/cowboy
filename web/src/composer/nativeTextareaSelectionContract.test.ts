import { assertEquals } from "jsr:@std/assert";

const textareaSource = await Deno.readTextFile(
  new URL("../ComposerTextarea.tsx", import.meta.url),
);
const editorSource = await Deno.readTextFile(
  new URL("../ComposerEditor.tsx", import.meta.url),
);

Deno.test("native textarea keeps React rerenders from replacing iOS selection", () => {
  assertEquals(textareaSource.includes('<TextField'), false);
  assertEquals(textareaSource.includes('component="textarea"'), true);
  assertEquals(textareaSource.includes('rows={1}'), true);
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
  assertEquals(textareaSource.includes("if (composingRef.current) return;"), true);
  assertEquals(textareaSource.includes("onCompositionUpdate"), true);
});

Deno.test("CM6 backspace does not steal iOS IME composition deletes", () => {
  assertEquals(editorSource.includes("isImeProtectedInput(e, view.composing)"), true);
  assertEquals(
    editorSource.includes("view.composing ? false : backspaceChain(view)"),
    true,
  );
});

Deno.test("ordinary native input bypasses MUI trailing-newline selection rewrites", () => {
  const inputStart = textareaSource.indexOf('defaultValue={value}');
  const inputEnd = textareaSource.indexOf('onSelect=', inputStart);
  const ordinaryInput = textareaSource.slice(inputStart, inputEnd);
  assertEquals(inputStart >= 0, true);
  assertEquals(inputEnd > inputStart, true);
  assertEquals(ordinaryInput.includes("setSelectionRange"), false);
  assertEquals(ordinaryInput.includes("TextareaAutosize"), false);
});

Deno.test("native textarea and CM6 expose the same logical selection handoff", () => {
  assertEquals(textareaSource.includes("getValue: (): string =>"), true);
  assertEquals(textareaSource.includes("getSelection: ()"), true);
  assertEquals(
    textareaSource.includes('selectionDirection === "backward"'),
    true,
  );
  assertEquals(textareaSource.includes("focusSelection: (selection"), true);
  assertEquals(editorSource.includes("getValue: (): string =>"), true);
  assertEquals(editorSource.includes("getSelection: ()"), true);
  assertEquals(editorSource.includes("focusSelection: (selection"), true);
  assertEquals(editorSource.includes("scrollIntoView: true"), true);
  assertEquals(textareaSource.includes("insertText: ("), true);
  assertEquals(editorSource.includes("insertText: ("), true);
  assertEquals(textareaSource.includes("replaceNativeSelection"), true);
  assertEquals(textareaSource.includes("lastSelectionRef"), true);
  assertEquals(textareaSource.includes("rememberedSelection(ta)"), true);
});
