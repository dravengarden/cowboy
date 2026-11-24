import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { confirmEnterIntent, desktopConfirmEnterIntent } from "./confirmShortcut";
import { isMac } from "./platform";

function keyEvent(
  overrides: Partial<KeyboardEvent> = {},
): KeyboardEvent {
  return {
    key: "Enter",
    shiftKey: false,
    repeat: false,
    isComposing: false,
    keyCode: 13,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    ...overrides,
  } as KeyboardEvent;
}

test("confirm modals suppress bare Enter", () => {
  assertEquals(confirmEnterIntent(keyEvent()), "suppress");
});

test("confirm modals accept only the platform Command chord", () => {
  const event = isMac
    ? keyEvent({ metaKey: true })
    : keyEvent({ ctrlKey: true });
  assertEquals(confirmEnterIntent(event), "confirm");
  assertEquals(confirmEnterIntent(keyEvent({ shiftKey: true })), "ignore");
  assertEquals(confirmEnterIntent(keyEvent({ isComposing: true })), "ignore");
  assertEquals(confirmEnterIntent(keyEvent({ keyCode: 229 })), "ignore");
});

test("desktopConfirmEnterIntent accepts idle CJK Mod+Enter and ignores composition", () => {
  const mod = isMac ? { metaKey: true } : { ctrlKey: true };
  assertEquals(
    desktopConfirmEnterIntent(
      keyEvent({ key: "Process", code: "Enter", keyCode: 229, ...mod }),
      { composing: false },
    ),
    "confirm",
  );
  assertEquals(
    desktopConfirmEnterIntent(
      keyEvent({ key: "Process", code: "Enter", keyCode: 229, isComposing: true, ...mod }),
      { composing: false },
    ),
    "ignore",
  );
  assertEquals(
    desktopConfirmEnterIntent(
      keyEvent({ key: "Enter", code: "Enter", ...mod }),
      { composing: true },
    ),
    "ignore",
  );
});
