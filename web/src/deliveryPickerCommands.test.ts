import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { deliveryPickerKey } from "./deliveryPickerCommands";

const key = (
  value: string,
  fields: Partial<KeyboardEvent> = {},
): KeyboardEvent =>
  ({
    key: value,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    repeat: false,
    isComposing: false,
    keyCode: 0,
    ...fields,
  }) as KeyboardEvent;

test("picker keyboard activation excludes IME, repeats and shifted Return", () => {
  for (const fields of [{ repeat: true }, { shiftKey: true }]) {
    assertEquals(deliveryPickerKey(key("Enter", fields)), "consume");
  }
  for (const fields of [{ isComposing: true }, { keyCode: 229 }]) {
    assertEquals(deliveryPickerKey(key("Enter", fields)), null);
  }
  assertEquals(deliveryPickerKey(key("Enter")), "activate");
  assertEquals(deliveryPickerKey(key("Enter", { metaKey: true })), "activate");
  assertEquals(deliveryPickerKey(key("Enter", { ctrlKey: true })), "activate");
  assertEquals(deliveryPickerKey(key("Enter", { altKey: true })), "consume");
});

test("scoped navigation works without the desktop leader", () => {
  assertEquals(deliveryPickerKey(key("j")), "next");
  assertEquals(deliveryPickerKey(key("k")), "previous");
  assertEquals(deliveryPickerKey(key("ArrowDown")), "next");
  assertEquals(deliveryPickerKey(key("Escape")), "close");
  assertEquals(deliveryPickerKey(key(".", { metaKey: true })), "stop");
  assertEquals(deliveryPickerKey(key("j", { ctrlKey: true })), null);
});
