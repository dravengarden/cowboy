import { assertEquals } from "jsr:@std/assert";
import { addShellPathBreaks } from "./shellFormatter.ts";

Deno.test("shell path wrapping prefers separators without changing visible text", () => {
  const source = "BRIDGE=/home/draven/chrome-debug-bridge/helpers/bridge.sh";
  const display = addShellPathBreaks(source);

  assertEquals(display.replaceAll("\u200b", ""), source);
  assertEquals(display, "BRIDGE=/\u200bhome/\u200bdraven/\u200bchrome-debug-bridge/\u200bhelpers/\u200bbridge.sh");
});
