import { test } from "bun:test";
import { readFileSync } from "node:fs";
import { assertEquals } from "../../../tools/test-assert.ts";

test("DesktopPromptCompanionLayout keeps a single stable tree so Prompt/Draft editors never remount on layout changes", () => {
  const src = readFileSync(new URL("./DesktopPromptCompanionRail.tsx", import.meta.url), "utf8");
  const fnBody = src.slice(src.indexOf("export function DesktopPromptCompanionLayout"));
  // Must render `{children}` in exactly one stable slot (never branched by `if (split)`).
  assertEquals(fnBody.match(/\{children\}/g)?.length ?? 0, 1);
  assertEquals(fnBody.match(/\{planSlot\}/g)?.length ?? 0, 1);
  assertEquals(fnBody.match(/\{pendingSlot\}/g)?.length ?? 0, 1);
  assertEquals(fnBody.includes("data-desktop-prompt-canvas-slot"), true);
  assertEquals(src.includes("OPEN_BACKGROUND_TASKS_EVENT"), true);
});
