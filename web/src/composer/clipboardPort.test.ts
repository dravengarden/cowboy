import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  createClipboardPort,
  hasNativeClipboardBridge,
  webClipboardPort,
} from "./clipboardPort.ts";

test("clipboard port picks native only when a pasteboard bridge exists", () => {
  assertEquals(hasNativeClipboardBridge({}), false);
  assertEquals(
    hasNativeClipboardBridge({ __cowboyReadClipboard: () => Promise.resolve("") }),
    true,
  );
  assertEquals(createClipboardPort().surface, "web");
});

test("web clipboard port offers Paste without probing the pasteboard", async () => {
  const previous = globalThis.navigator;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      clipboard: {
        readText: () => Promise.resolve("from-web"),
      },
    },
  });
  try {
    const port = webClipboardPort();
    assertEquals(port.surface, "web");
    assertEquals(await port.status(), {
      surface: "web",
      pasteAvailable: true,
      stageImagesFirst: false,
      imageCount: 0,
    });
    assertEquals(await port.read(), { text: "from-web", files: [] });
  } finally {
    Object.defineProperty(globalThis, "navigator", {
      configurable: true,
      value: previous,
    });
  }
});
