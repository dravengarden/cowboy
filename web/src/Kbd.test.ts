import { assertEquals } from "jsr:@std/assert";
import { confirmEnterIntent } from "./confirmShortcut";
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

Deno.test("confirm modals suppress bare Enter", () => {
  assertEquals(confirmEnterIntent(keyEvent()), "suppress");
});

Deno.test("confirm modals accept only the platform Command chord", () => {
  const event = isMac
    ? keyEvent({ metaKey: true })
    : keyEvent({ ctrlKey: true });
  assertEquals(confirmEnterIntent(event), "confirm");
  assertEquals(confirmEnterIntent(keyEvent({ shiftKey: true })), "ignore");
  assertEquals(confirmEnterIntent(keyEvent({ isComposing: true })), "ignore");
});
