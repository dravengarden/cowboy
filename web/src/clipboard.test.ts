import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";

const textareaSource = await readFile(
  new URL("./ComposerTextarea.tsx", import.meta.url), "utf8",
);
const clipboardSource = await readFile(
  new URL("./clipboard.ts", import.meta.url), "utf8",
);
const nativeShellSource = await readFile(
  new URL("./nativeShell.ts", import.meta.url), "utf8",
);
const formatActionsSource = await readFile(
  new URL("./MobileComposerFormatActions.tsx", import.meta.url), "utf8",
);

test("mobile paste stays on UIKit's native edit-menu path", () => {
  assertEquals(textareaSource.includes("onPaste={(e)"), true);
  assertEquals(textareaSource.includes('addEventListener("touchstart"'), false);
  assertEquals(textareaSource.includes("navigator.clipboard"), false);
  assertEquals(textareaSource.includes("readWebClipboard"), true);
  assertEquals(textareaSource.includes("blankPaste"), false);
  assertEquals(clipboardSource.includes("readComposerClipboard"), false);
});

test("explicit dock paste uses the platform clipboard port", () => {
  assertEquals(
    nativeShellSource.includes("__cowboyClipboardImageStatus"),
    true,
  );
  assertEquals(
    nativeShellSource.includes("__cowboyReadClipboardImages"),
    true,
  );
  assertEquals(nativeShellSource.includes("__cowboyReadClipboard"), true);
  assertEquals(nativeShellSource.includes("navigator.clipboard.read("), false);
  assertEquals(formatActionsSource.includes('title="Paste"'), true);
  assertEquals(formatActionsSource.includes("createClipboardPort"), true);
  assertEquals(formatActionsSource.includes("clipboardPort.read()"), true);
  assertEquals(
    formatActionsSource.includes("insertText(contents.text, selection)"),
    true,
  );
  assertEquals(formatActionsSource.includes("contents.text.length > 0"), true);
  assertEquals(
    formatActionsSource.includes("setInterval(refreshVisible, 1000)"),
    true,
  );
  assertEquals(
    formatActionsSource.includes(
      "capturedSelectionRef.current ??\n      editorRef.current?.getSelection()",
    ),
    true,
  );
  assertEquals(
    formatActionsSource.includes(
      "useReliableTouchTap<HTMLButtonElement>",
    ),
    true,
  );
  assertEquals(formatActionsSource.includes("pasteTap.onPointerUp"), true);
  assertEquals(formatActionsSource.includes("pasteTap.onClick"), true);
  assertEquals(formatActionsSource.includes("mobile_paste_started"), true);
  assertEquals(formatActionsSource.includes("mobile_paste_finished"), true);
});

test("copyRichText writes dual text/plain + text/html via ClipboardItem and falls back cleanly", async () => {
  const { copyRichText } = await import("./clipboard.ts");
  const written: unknown[] = [];
  const origNav = globalThis.navigator;
  const origItem = globalThis.ClipboardItem;
  class FakeClipboardItem {
    readonly items: Record<string, Blob>;
    constructor(items: Record<string, Blob>) {
      this.items = items;
    }
  }
  try {
    Object.defineProperty(globalThis, "ClipboardItem", {
      value: FakeClipboardItem,
      configurable: true,
    });
    Object.defineProperty(globalThis, "navigator", {
      value: {
        clipboard: {
          write: async (items: unknown[]) => {
            written.push(...items);
          },
          writeText: async () => {},
        },
      },
      configurable: true,
    });
    const ok = await copyRichText("Hello world", "<p><strong>Hello</strong> world</p>");
    assertEquals(ok, true);
    assertEquals(written.length, 1);
    const entry = written[0] as FakeClipboardItem;
    assertEquals(await entry.items["text/plain"]!.text(), "Hello world");
    assertEquals(await entry.items["text/html"]!.text(), "<p><strong>Hello</strong> world</p>");

    // Fallback to writeText when clipboard.write rejects
    let fallbackText = "";
    Object.defineProperty(globalThis, "navigator", {
      value: {
        clipboard: {
          write: async () => { throw new Error("not allowed"); },
          writeText: async (text: string) => { fallbackText = text; },
        },
      },
      configurable: true,
    });
    const fallbackOk = await copyRichText("Fallback plain", "<p>Fallback plain</p>");
    assertEquals(fallbackOk, true);
    assertEquals(fallbackText, "Fallback plain");
  } finally {
    Object.defineProperty(globalThis, "navigator", { value: origNav, configurable: true });
    Object.defineProperty(globalThis, "ClipboardItem", { value: origItem, configurable: true });
  }
});

test("copyRichTextDetailed pre-checks ClipboardItem.supports synchronously and signals retryPlainText when activation expires", async () => {
  const { copyRichTextDetailed } = await import("./clipboard.ts");
  const origNav = globalThis.navigator;
  const origItem = globalThis.ClipboardItem;
  let writeCalled = 0;
  let plainWritten = "";
  class UnsupportedHtmlClipboardItem {
    static supports(type: string): boolean {
      return type === "text/plain";
    }
    constructor(readonly items: Record<string, Blob>) {}
  }
  try {
    Object.defineProperty(globalThis, "ClipboardItem", {
      value: UnsupportedHtmlClipboardItem,
      configurable: true,
    });
    Object.defineProperty(globalThis, "navigator", {
      value: {
        clipboard: {
          write: async () => { writeCalled++; },
          writeText: async (text: string) => { plainWritten = text; },
        },
      },
      configurable: true,
    });
    const skipped = await copyRichTextDetailed("Sync plain", "<p>Sync plain</p>");
    assertEquals(skipped, { ok: true, retryPlainText: false });
    assertEquals(writeCalled, 0);
    assertEquals(plainWritten, "Sync plain");

    // When write throws AND plain fallback throws (expired iOS user activation), signal retryPlainText
    class SupportedClipboardItem {
      static supports(): boolean { return true; }
      constructor(readonly items: Record<string, Blob>) {}
    }
    Object.defineProperty(globalThis, "ClipboardItem", {
      value: SupportedClipboardItem,
      configurable: true,
    });
    Object.defineProperty(globalThis, "navigator", {
      value: {
        clipboard: {
          write: async () => { throw new Error("NotAllowedError"); },
          writeText: async () => { throw new Error("NotAllowedError"); },
        },
      },
      configurable: true,
    });
    const failed = await copyRichTextDetailed("Retry me", "<p>Retry me</p>");
    assertEquals(failed, { ok: false, retryPlainText: true });
  } finally {
    Object.defineProperty(globalThis, "navigator", { value: origNav, configurable: true });
    Object.defineProperty(globalThis, "ClipboardItem", { value: origItem, configurable: true });
  }
});

test("superseded copyRichTextDetailed rejection never overwrites a newer copyText write", async () => {
  const { copyRichTextDetailed, copyText } = await import("./clipboard.ts");
  const origNav = globalThis.navigator;
  const origItem = globalThis.ClipboardItem;
  let rejectRich!: (err: Error) => void;
  const writes: string[] = [];
  class SupportedClipboardItem {
    static supports(): boolean { return true; }
    constructor(readonly items: Record<string, Blob>) {}
  }
  try {
    Object.defineProperty(globalThis, "ClipboardItem", {
      value: SupportedClipboardItem,
      configurable: true,
    });
    Object.defineProperty(globalThis, "navigator", {
      value: {
        clipboard: {
          write: () => new Promise<void>((_, reject) => { rejectRich = reject; }),
          writeText: async (text: string) => { writes.push(text); },
        },
      },
      configurable: true,
    });
    const pendingRich = copyRichTextDetailed("OLD_PLAIN", "<p>OLD</p>");
    const mdOk = await copyText("**NEW_MARKDOWN**");
    assertEquals(mdOk, true);
    assertEquals(writes, ["**NEW_MARKDOWN**"]);
    rejectRich(new Error("late failure"));
    const richRes = await pendingRich;
    assertEquals(richRes.ok, false);
    assertEquals(richRes.superseded, true);
    assertEquals(writes, ["**NEW_MARKDOWN**"]);
  } finally {
    Object.defineProperty(globalThis, "navigator", { value: origNav, configurable: true });
    Object.defineProperty(globalThis, "ClipboardItem", { value: origItem, configurable: true });
  }
});
