import { test } from "bun:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { assertEquals } from "../../../tools/test-assert.ts";
import { chromeShortcutConflict } from "../desktop/commands/chromeShortcutPolicy";
import { desktopBrowserReleaseShortcut } from "../desktop/commands/workspaceShortcuts";
import { browserViewport } from "./api";
import { DesktopBrowserHeader, releaseLabel } from "./DesktopBrowserModal";
import {
  MAXIMIZED_KEY,
  escapeClosesModal,
  headerQuickLinks,
  isBrowserRelease,
  modalBox,
  modalFrameSize,
  readMaximized,
  writeMaximized,
} from "./desktopBrowserLayout";
import { quickLinks } from "./quickLinks";

function render(element: React.ReactElement): string {
  const prior = Object.getOwnPropertyDescriptor(globalThis, "React");
  Object.defineProperty(globalThis, "React", { configurable: true, value: React });
  try {
    return renderToStaticMarkup(element);
  } finally {
    if (prior) Object.defineProperty(globalThis, "React", prior);
    else Reflect.deleteProperty(globalThis, "React");
  }
}

const POINTER = { touch: false, portrait: false };

function mode(window: { width: number; height: number }, maximized: boolean): string {
  const frame = modalFrameSize(window, maximized);
  const viewport = browserViewport(frame.width, frame.height, POINTER);
  return `${viewport.width}x${viewport.height}@${viewport.scale}`;
}

test("the modal picks the largest fixed mode its frame fits, else the smallest scaled", () => {
  // A 13" laptop: the default modal fits 1280x800.
  assertEquals(mode({ width: 1440, height: 900 }, false), "1280x800@1");
  // A 1080p monitor: 1440x900 fits the default modal, 1920x1080 does not.
  assertEquals(mode({ width: 1920, height: 1080 }, false), "1440x900@1");
  // Maximized on a 1920x1200 monitor: 1920x1080 fits under the header.
  assertEquals(mode({ width: 1920, height: 1200 }, true), "1920x1080@1");
  // A large display: the default box (bounded) still fits 1920x1080.
  assertEquals(mode({ width: 2560, height: 1440 }, false), "1920x1080@1");
  // A small window: nothing fits, the smallest mode is scaled down to fit.
  assertEquals(mode({ width: 1280, height: 720 }, false), "1280x800@1");
  // Only the fixed modes are ever requested (no arbitrary geometry).
  for (const width of [900, 1300, 1500, 2000, 3000]) {
    for (const height of [600, 850, 950, 1150, 1700]) {
      const viewport = browserViewport(width, height, POINTER);
      assertEquals(
        ["1280x800", "1440x900", "1920x1080"].includes(`${viewport.width}x${viewport.height}`),
        true,
      );
    }
  }
  assertEquals(modalBox(true), { width: "100vw", height: "100vh", borderRadius: 0 });
  assertEquals(modalBox(false).borderRadius > 0, true);
});

test("the maximize choice is remembered, and a failing storage is harmless", () => {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => void values.set(key, value),
  };
  assertEquals(readMaximized(storage), false);
  writeMaximized(storage, true);
  assertEquals(values.get(MAXIMIZED_KEY), "1");
  assertEquals(readMaximized(storage), true);
  writeMaximized(storage, false);
  assertEquals(readMaximized(storage), false);
  const broken = {
    getItem: (): string | null => {
      throw new Error("denied");
    },
    setItem: () => {
      throw new Error("denied");
    },
  };
  assertEquals(readMaximized(broken), false);
  writeMaximized(broken, true);
  assertEquals(readMaximized(undefined), false);
});

test("Esc closes the modal only from Cowboy's chrome, never from the remote frame", () => {
  const element = (inFrame: boolean) => ({
    closest: (selector: string) =>
      inFrame && selector === "[data-machine-browser-viewer]" ? {} : null,
  });
  assertEquals(escapeClosesModal(element(true) as unknown as Element), false);
  assertEquals(escapeClosesModal(element(false) as unknown as Element), true);
  assertEquals(escapeClosesModal(null), true);
});

test("the release chord is registered, browser-safe and IME-safe", () => {
  assertEquals(desktopBrowserReleaseShortcut(true), "Mod+Shift+Escape");
  assertEquals(desktopBrowserReleaseShortcut(false), "Alt+Shift+Q");
  for (const mac of [true, false]) {
    assertEquals(
      chromeShortcutConflict("browser.release", desktopBrowserReleaseShortcut(mac), mac),
      null,
    );
  }
  const key = (fields: Partial<KeyboardEvent>) =>
    ({
      key: "Escape",
      code: "Escape",
      metaKey: true,
      ctrlKey: false,
      shiftKey: true,
      altKey: false,
      isComposing: false,
      keyCode: 27,
      target: null,
      ...fields,
    }) as KeyboardEvent;
  assertEquals(isBrowserRelease(key({}), "Mod+Shift+Escape", true), true);
  // Plain Esc stays the remote page's.
  assertEquals(
    isBrowserRelease(key({ metaKey: false, shiftKey: false }), "Mod+Shift+Escape", true),
    false,
  );
  // Never during an IME composition.
  assertEquals(isBrowserRelease(key({ isComposing: true }), "Mod+Shift+Escape", true), false);
  assertEquals(
    isBrowserRelease(
      key({ key: "Q", code: "KeyQ", metaKey: false, altKey: true }),
      "Alt+Shift+Q",
      false,
    ),
    true,
  );
  assertEquals(releaseLabel("Alt+Shift+Q").endsWith("Q"), true);
});

function header(shared: boolean, maximized = false): string {
  return render(
    React.createElement(DesktopBrowserHeader, {
      labelId: "browser-label",
      identity: {
        accountLabel: shared ? "Google · shared with Lynx, Pika, Fox" : "DeepSeek · Mole",
        machineLabel: "OVH Matrix",
        status: "Hibernated",
        shared,
      },
      chrome: {
        navigation: false,
        keyboard: false,
        address: false,
        detach: false,
        hibernate: false,
        snapshot: false,
        restart: false,
        reset: false,
        paste: false,
        copy: false,
        refresh: true,
      },
      onChord: () => {},
      quickLinks: quickLinks(shared ? "openai" : "deepseek", shared),
      quickDisabled: false,
      onQuick: () => {},
      menu: React.createElement("button", { "aria-label": "More browser actions" }),
      maximized,
      onToggleMaximized: () => {},
      onClose: () => {},
    }),
  );
}

test("the header row names the browser, its state and its compact quick links", () => {
  const own = header(false);
  assertEquals(own.includes('id="browser-label"'), true);
  assertEquals(own.includes("DeepSeek · Mole"), true);
  assertEquals(own.includes("Hibernated"), true);
  assertEquals(own.includes('data-machine-browser-identity="own"'), true);
  for (const label of ["Back", "Forward", "Reload", "Maximize", "Close browser"]) {
    assertEquals(own.includes(`aria-label="${label}"`), true, label);
  }
  // Three chips inline, the card's own service first; the rest in More.
  assertEquals((own.match(/aria-label="Open [^"]+ in this browser"/g) ?? []).length, 3);
  assertEquals(own.indexOf("Open DeepSeek in this browser") < own.indexOf("Open Claude"), true);
  assertEquals(
    headerQuickLinks(quickLinks("deepseek", false)).overflow.map((link) => link.key),
    ["xai", "google"],
  );
  const shared = header(true, true);
  assertEquals(shared.includes('data-machine-browser-identity="shared"'), true);
  assertEquals(shared.includes('aria-label="Shared browser"'), true);
  assertEquals(shared.includes("Google · shared with Lynx, Pika, Fox"), true);
  assertEquals(shared.includes('aria-label="Restore"'), true);
  assertEquals(shared.includes('aria-pressed="true"'), true);
});
