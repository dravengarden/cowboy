import { assertEquals } from "jsr:@std/assert";
import { desktopBrowserChromeShortcut } from "./desktopBrowserChrome.ts";

function key(
  init: {
    key: string;
    code?: string;
    metaKey?: boolean;
    ctrlKey?: boolean;
    altKey?: boolean;
    shiftKey?: boolean;
  },
) {
  return {
    key: init.key,
    code: init.code,
    metaKey: init.metaKey ?? false,
    ctrlKey: init.ctrlKey ?? false,
    altKey: init.altKey ?? false,
    shiftKey: init.shiftKey ?? false,
  };
}

Deno.test("Desktop swallows Chrome Find and related browser chrome chords", () => {
  assertEquals(
    desktopBrowserChromeShortcut(
      key({ key: "f", code: "KeyF", metaKey: true }),
      true,
    ),
    true,
  );
  assertEquals(
    desktopBrowserChromeShortcut(
      key({ key: "f", code: "KeyF", ctrlKey: true }),
      false,
    ),
    true,
  );
  assertEquals(
    desktopBrowserChromeShortcut(
      key({ key: "g", code: "KeyG", metaKey: true }),
      true,
    ),
    true,
  );
  assertEquals(
    desktopBrowserChromeShortcut(
      key({ key: "o", code: "KeyO", ctrlKey: true }),
      false,
    ),
    true,
  );
  assertEquals(
    desktopBrowserChromeShortcut(
      key({ key: "j", code: "KeyJ", metaKey: true }),
      true,
    ),
    true,
  );
});

Deno.test("Desktop leaves OS, edit, reload, and word-motion chords alone", () => {
  assertEquals(
    desktopBrowserChromeShortcut(
      key({ key: "c", code: "KeyC", metaKey: true }),
      true,
    ),
    false,
  );
  assertEquals(
    desktopBrowserChromeShortcut(
      key({ key: "z", code: "KeyZ", metaKey: true }),
      true,
    ),
    false,
  );
  assertEquals(
    desktopBrowserChromeShortcut(
      key({ key: "r", code: "KeyR", metaKey: true }),
      true,
    ),
    false,
  );
  assertEquals(
    desktopBrowserChromeShortcut(
      key({ key: "w", code: "KeyW", metaKey: true }),
      true,
    ),
    false,
  );
  assertEquals(
    desktopBrowserChromeShortcut(
      key({ key: "h", code: "KeyH", metaKey: true }),
      true,
    ),
    false,
  );
  assertEquals(
    desktopBrowserChromeShortcut(
      key({ key: "ArrowLeft", code: "ArrowLeft", altKey: true }),
      true,
    ),
    false,
  );
});

Deno.test("Windows Alt arrows are shielded as browser history, not Mac Option word motion", () => {
  assertEquals(
    desktopBrowserChromeShortcut(
      key({ key: "ArrowLeft", code: "ArrowLeft", altKey: true }),
      false,
    ),
    true,
  );
  assertEquals(
    desktopBrowserChromeShortcut(
      key({ key: "ArrowRight", code: "ArrowRight", altKey: true }),
      false,
    ),
    true,
  );
});
