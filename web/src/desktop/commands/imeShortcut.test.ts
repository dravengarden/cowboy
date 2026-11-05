import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import { desktopImeOwnsKey, shortcutKey } from "./imeShortcut.ts";

const appSource = await readFile(
  new URL("../../App.tsx", import.meta.url), "utf8",
);
const hostSource = await readFile(
  new URL("./DesktopCommandHost.tsx", import.meta.url), "utf8",
);
const providerSource = await readFile(
  new URL("./DesktopCommandProvider.tsx", import.meta.url), "utf8",
);
const topBarSource = await readFile(
  new URL("../DesktopTopBarControls.tsx", import.meta.url), "utf8",
);
const transcriptSource = await readFile(
  new URL("../../Transcript.tsx", import.meta.url), "utf8",
);
const exploreSource = await readFile(
  new URL("../../explore/ExploreSurface.tsx", import.meta.url), "utf8",
);

class FakeElement {
  constructor(private readonly selectors: readonly string[]) {}
  matches(selector: string): boolean {
    return this.selectors.some((own) => selector.includes(own));
  }
  getAttribute(): string | null {
    return null;
  }
}

// Bun has no DOM; the classifier only needs `instanceof Element` + matches().
(globalThis as { Element?: unknown }).Element = FakeElement;

const BUTTON = new FakeElement(["button"]) as unknown as EventTarget;
const INPUT = new FakeElement(["input"]) as unknown as EventTarget;
const VIM_SINK = new FakeElement(["[data-vim-command-sink]"]) as unknown as EventTarget;

function key(
  overrides: Partial<KeyboardEvent> = {},
): KeyboardEvent {
  return {
    isComposing: false,
    key: "f",
    code: "KeyF",
    keyCode: 70,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    target: BUTTON,
    ...overrides,
  } as KeyboardEvent;
}

const IDLE_CJK = { key: "Process", keyCode: 229 } as const;

test("an idle CJK source cannot disable a shortcut on dialog chrome", () => {
  // The Session dialog's `F` (Replace or fork) under macOS Pinyin.
  assertEquals(desktopImeOwnsKey(key(IDLE_CJK)), false);
  assertEquals(shortcutKey(key(IDLE_CJK)), "f");
  assertEquals(shortcutKey(key({ ...IDLE_CJK, code: "KeyG", shiftKey: true })), "G");
  assertEquals(shortcutKey(key({ ...IDLE_CJK, code: "Comma" })), ",");
  for (const code of ["Enter", "ArrowDown", "Escape", "Home"]) {
    assertEquals(shortcutKey(key({ ...IDLE_CJK, code })), code);
  }
  assertEquals(shortcutKey(key({ ...IDLE_CJK, code: "Space" })), " ");
  assertEquals(shortcutKey(key({ ...IDLE_CJK, code: "Digit2" })), "2");
  assertEquals(shortcutKey(key({ key: "Dead", code: "KeyJ" })), "j");
});

test("the Vim Normal sink resolves IME-labelled keys physically", () => {
  assertEquals(desktopImeOwnsKey(key({ ...IDLE_CJK, target: VIM_SINK })), false);
});

test("text fields and real compositions keep IME keys", () => {
  for (const event of [
    key({ ...IDLE_CJK, target: INPUT }),
    key({ key: "Dead", target: INPUT }),
    key({ isComposing: true }),
    key({ isComposing: true, target: VIM_SINK }),
  ]) {
    assertEquals(desktopImeOwnsKey(event), true);
    assertEquals(shortcutKey(event), null);
  }
  // Plain typing in a field is text, not an IME key.
  assertEquals(desktopImeOwnsKey(key({ target: INPUT })), false);
});

test("an idle CJK source keeps modified chords", () => {
  assertEquals(desktopImeOwnsKey(key({ ...IDLE_CJK, code: "KeyK", metaKey: true, target: INPUT })), false);
});

test("every Desktop shortcut owner checks the shared IME boundary", () => {
  for (
    const [name, source, minimum] of [
      ["session surfaces", appSource, 3],
      ["command palette", hostSource, 1],
      ["workspace provider", providerSource, 1],
      ["top bar overlays", topBarSource, 2],
      ["transcript inspector", transcriptSource, 1],
      ["question page navigation", exploreSource, 2],
    ] as const
  ) {
    const checks = source.match(/desktopImeOwnsKey\(/gu)?.length ?? 0;
    assert(
      checks >= minimum,
      name + " has " + String(checks) + " IME checks; expected " +
        String(minimum),
    );
  }
});
