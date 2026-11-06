import { assert, assertEquals } from "../../../tools/test-assert.ts";
import { test } from "bun:test";
import { EDITOR_PLUGIN_SANDBOX_BOOT } from "./sandboxBoot";

test("the sandbox boot text survives as one inline block", () => {
  // The closing tag would end the block early, and the server would hash less
  // than the frame runs.
  assertEquals(/<\/script/i.test(EDITOR_PLUGIN_SANDBOX_BOOT), false);
  assert(EDITOR_PLUGIN_SANDBOX_BOOT.includes("cowboy-sandbox-ready"));
});
