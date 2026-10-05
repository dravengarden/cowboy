import { strict as assert } from "node:assert";
import {
  desktopKeyIntent,
  type DesktopKeyIntentEvent,
  physicalCommandKey,
} from "./keyIntent";

class FakeElement {
  constructor(private readonly selectors: readonly string[]) {}
  matches(selector: string): boolean {
    return this.selectors.some((own) => selector.includes(own));
  }
}

// Deno has no DOM; the classifier only needs `instanceof Element` + matches().
(globalThis as { Element?: unknown }).Element = FakeElement;

function target(...selectors: string[]): EventTarget {
  return new FakeElement(selectors) as unknown as EventTarget;
}

const INPUT = (): EventTarget => target("input");
const BUTTON = (): EventTarget => target("button");
const VIM_SINK = (): EventTarget => target("[data-vim-command-sink]");

function key(
  code: string,
  overrides: Partial<DesktopKeyIntentEvent> = {},
): DesktopKeyIntentEvent {
  return {
    code,
    key: code.replace(/^Key/, "").toLowerCase(),
    keyCode: 0,
    isComposing: false,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    target: BUTTON(),
    ...overrides,
  };
}

const idle = { composing: false };

Deno.test("composition owns every key, including modified chords and Esc", () => {
  for (const event of [
    key("KeyJ", { isComposing: true }),
    key("Escape", { key: "Escape", isComposing: true, target: INPUT() }),
    key("BracketRight", { metaKey: true, isComposing: true }),
  ]) {
    assert.deepEqual(desktopKeyIntent(event, idle), { owner: "ime" });
  }
  assert.deepEqual(
    desktopKeyIntent(key("KeyL"), { composing: true }),
    { owner: "ime" },
  );
});

Deno.test("text fields keep unmodified keys and IME markers", () => {
  assert.deepEqual(
    desktopKeyIntent(key("KeyJ", { target: INPUT() }), idle),
    { owner: "text", key: "j" },
  );
  assert.deepEqual(
    desktopKeyIntent(
      key("KeyJ", { key: "Process", keyCode: 229, target: INPUT() }),
      idle,
    ),
    { owner: "ime" },
  );
  assert.deepEqual(
    desktopKeyIntent(key("Escape", { key: "Escape", target: INPUT() }), idle),
    { owner: "text", key: "Escape" },
  );
});

Deno.test("idle CJK sources keep modified chords as physical commands", () => {
  assert.deepEqual(
    desktopKeyIntent(
      key("BracketLeft", {
        key: "Process",
        keyCode: 229,
        ctrlKey: true,
        target: INPUT(),
      }),
      idle,
    ),
    { owner: "command", key: "[", modified: true },
  );
});

Deno.test("non-editable chrome and the Vim sink resolve physical keys", () => {
  for (const make of [BUTTON, VIM_SINK]) {
    assert.deepEqual(
      desktopKeyIntent(
        key("KeyL", { key: "Process", keyCode: 229, target: make() }),
        idle,
      ),
      { owner: "command", key: "l", modified: false },
    );
  }
  assert.deepEqual(
    desktopKeyIntent(key("Digit2", { key: "２", target: BUTTON() }), idle),
    { owner: "command", key: "2", modified: false },
  );
});

Deno.test("physical keys cover the number row without breaking shifted symbols", () => {
  assert.equal(physicalCommandKey({ code: "Digit3", key: "3", shiftKey: false }), "3");
  assert.equal(physicalCommandKey({ code: "Numpad1", key: "1", shiftKey: false }), "1");
  assert.equal(physicalCommandKey({ code: "Digit1", key: "!", shiftKey: true }), "!");
  assert.equal(physicalCommandKey({ code: "KeyG", key: "Process", shiftKey: true }), "G");
});
