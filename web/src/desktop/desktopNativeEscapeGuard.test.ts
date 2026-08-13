import { assertEquals } from "jsr:@std/assert";
import { desktopEscapeGuardAction } from "./desktopNativeEscapeGuard.ts";

const input = {
  key: "Escape",
  ime: false,
  modalOpen: false,
  editorOwnsEscape: false,
};

Deno.test("Desktop preclaims ordinary Escape from the native window", () => {
  assertEquals(desktopEscapeGuardAction(input), "prevent-native");
});

Deno.test("Desktop preclaims modal Escape even over a stale editor focus", () => {
  assertEquals(
    desktopEscapeGuardAction({
      ...input,
      modalOpen: true,
      editorOwnsEscape: true,
    }),
    "prevent-native",
  );
});

Deno.test("Desktop leaves an unmodified editor Escape to CodeMirror and Vim", () => {
  assertEquals(
    desktopEscapeGuardAction({ ...input, editorOwnsEscape: true }),
    "defer-to-editor",
  );
});

Deno.test("Desktop never claims IME or unrelated keys", () => {
  assertEquals(desktopEscapeGuardAction({ ...input, ime: true }), "ignore");
  assertEquals(desktopEscapeGuardAction({ ...input, key: "Enter" }), "ignore");
});
