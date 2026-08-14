import { assertEquals } from "jsr:@std/assert";
import {
  composerEnterPath,
  composerSoftwareKeyboardOpen,
  emptyComposerInputDebugRate,
  safeComposerDebugKey,
  shouldSampleComposerInputDebug,
  textareaLineMetrics,
} from "./composerInputDebugPolicy";

const source = await Deno.readTextFile(
  new URL("./composerInputDebug.ts", import.meta.url),
);

Deno.test("composer debug keys never include typed characters", () => {
  assertEquals(safeComposerDebugKey("Enter"), "Enter");
  assertEquals(safeComposerDebugKey("Backspace"), "Backspace");
  assertEquals(safeComposerDebugKey("a"), "char");
  assertEquals(safeComposerDebugKey("你"), "char");
  assertEquals(safeComposerDebugKey("Dead"), "other");
});

Deno.test("composer debug sampling is off until enabled and then rate limited", () => {
  const rate = emptyComposerInputDebugRate();
  assertEquals(shouldSampleComposerInputDebug(false, 1_000, rate).sample, false);
  let accepted = 0;
  let dropped = 0;
  for (let i = 0; i < 40; i++) {
    const result = shouldSampleComposerInputDebug(true, 5_000, rate);
    if (result.sample) accepted += 1;
    else dropped += 1;
  }
  assertEquals(accepted, 24);
  assertEquals(dropped, 16);
});

Deno.test("textarea debug metrics keep lengths and drop document text", () => {
  const value = "secret prompt ![img](cowboy-att:abc)\nnext";
  const metrics = textareaLineMetrics(value, 6);
  assertEquals(metrics.line, 1);
  assertEquals(metrics.lines, 2);
  assertEquals(metrics.lineLength, 36);
});

Deno.test("composer input debug never writes the native selection", () => {
  assertEquals(source.includes("setSelectionRange"), false);
  assertEquals(source.includes("removeAllRanges"), false);
  assertEquals(source.includes("addRange"), false);
  assertEquals(source.includes("drawSelection"), false);
  assertEquals(source.includes("composer_input_debug"), true);
  assertEquals(source.includes("input.inputType ?? keyEvent.key"), false);
  assertEquals(source.includes("input?.inputType ?? keyEvent?.key"), false);
});

Deno.test("debug mode labels software-keyboard Return separately from HID Enter", () => {
  assertEquals(composerSoftwareKeyboardOpen(500, 852), true);
  assertEquals(composerSoftwareKeyboardOpen(852, 852), false);
  assertEquals(composerEnterPath("insertLineBreak", "", false), "software");
  assertEquals(composerEnterPath("keydown", "Enter", false), "hardware_or_hid");
  assertEquals(composerEnterPath("keydown", "Enter", true), "software");
  assertEquals(composerEnterPath("insertText", "char", true), "unknown");
});

Deno.test("desktop and mobile settings expose the same debug mode toggle", async () => {
  const app = await Deno.readTextFile(new URL("../App.tsx", import.meta.url));
  assertEquals(app.includes('label="Debug mode"'), true);
  assertEquals(app.includes('ariaLabel="Toggle composer debug mode"'), true);
  assertEquals(app.includes('"aria-label": "Composer debug mode"'), true);
  assertEquals(app.includes("setComposerDebugSetting"), true);
  assertEquals(app.includes("reportComposerDebugModeChanged"), true);
});
