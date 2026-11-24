import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import {
  claimActiveVimOwner,
  clearBufferVimMode,
  getVimMode,
  setActiveVimOwner,
  setBufferVimMode,
  setVimMode,
} from "./vimModeStore.ts";

test("per-buffer Vim modes stay isolated and follow the active owner", () => {
  setActiveVimOwner("default");
  setVimMode("insert");
  assertEquals(getVimMode(), "insert");

  setBufferVimMode("queue:1", "normal");
  // Inactive buffer write must not overwrite the live mode.
  assertEquals(getVimMode(), "insert");

  setActiveVimOwner("queue:1");
  assertEquals(getVimMode(), "normal");

  setBufferVimMode("queue:1", "visual");
  assertEquals(getVimMode(), "visual");

  // Returning to default restores its preserved Insert mode.
  setActiveVimOwner("default");
  assertEquals(getVimMode(), "insert");

  clearBufferVimMode("queue:1");
  setVimMode("normal");
  assertEquals(getVimMode(), "normal");

  setVimMode("insert");
  const releaseDoc = claimActiveVimOwner("document:doc-1");
  setBufferVimMode("document:doc-1", "normal");
  assertEquals(getVimMode(), "normal");
  releaseDoc();
  assertEquals(getVimMode(), "insert");
  setVimMode("normal");
});
