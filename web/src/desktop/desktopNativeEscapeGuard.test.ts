import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { desktopEscapeGuardAction } from "./desktopNativeEscapeGuard.ts";

const input = {
  key: "Escape",
  ime: false,
  modalOpen: false,
  editorOwnsEscape: false,
};

test("Desktop preclaims ordinary Escape from the native window", () => {
  assertEquals(desktopEscapeGuardAction(input), "prevent-native");
});

test("Desktop preclaims modal Escape even over a stale editor focus", () => {
  assertEquals(
    desktopEscapeGuardAction({
      ...input,
      modalOpen: true,
      editorOwnsEscape: true,
    }),
    "prevent-native",
  );
});

test("Desktop leaves an unmodified editor Escape to CodeMirror and Vim", () => {
  assertEquals(
    desktopEscapeGuardAction({ ...input, editorOwnsEscape: true }),
    "defer-to-editor",
  );
});

test("Desktop never claims IME or unrelated keys", () => {
  assertEquals(desktopEscapeGuardAction({ ...input, ime: true }), "ignore");
  assertEquals(desktopEscapeGuardAction({ ...input, key: "Enter" }), "ignore");
});
