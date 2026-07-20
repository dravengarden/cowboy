import { assertEquals } from "jsr:@std/assert";
import { addRegexSoftBreaks, addShellPathBreaks } from "./shellFormatter.ts";

Deno.test("shell path wrapping prefers separators without changing visible text", () => {
  const source = "BRIDGE=/home/draven/chrome-debug-bridge/helpers/bridge.sh";
  const display = addShellPathBreaks(source);

  assertEquals(display.replaceAll("\u200b", ""), source);
  assertEquals(display, "BRIDGE=/\u200bhome/\u200bdraven/\u200bchrome-debug-bridge/\u200bhelpers/\u200bbridge.sh");
});

Deno.test("regex soft wrapping preserves escaped pipes and character classes", () => {
  const source = String.raw`alpha|beta\|literal|[a|b]`;
  const display = addRegexSoftBreaks(source);
  if (display !== `alpha|\u200bbeta\\|literal|\u200b[a|b]`) {
    throw new Error(`unexpected regex display: ${JSON.stringify(display)}`);
  }
  if (display.replaceAll("\u200b", "") !== source) {
    throw new Error("display-only regex breaks changed source bytes");
  }
});
