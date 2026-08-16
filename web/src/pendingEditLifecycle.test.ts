import { assertEquals } from "jsr:@std/assert";
import {
  pendingEditLiveText,
  pendingPanelDisclosureDecision,
  pendingRowVisibleText,
} from "./pendingEditLifecycle.ts";

Deno.test("pending panel disclosure expands and collapses outside editing", () => {
  assertEquals(
    pendingPanelDisclosureDecision({
      collapsed: true,
      editing: false,
      dirty: false,
    }),
    "expand",
  );
  assertEquals(
    pendingPanelDisclosureDecision({
      collapsed: false,
      editing: false,
      dirty: false,
    }),
    "collapse",
  );
});

Deno.test("pending panel disclosure never hides an unresolved edit", () => {
  assertEquals(
    pendingPanelDisclosureDecision({
      collapsed: false,
      editing: true,
      dirty: false,
    }),
    "discard-clean-edit-and-collapse",
  );
  assertEquals(
    pendingPanelDisclosureDecision({
      collapsed: false,
      editing: true,
      dirty: true,
    }),
    "confirm-dirty-edit",
  );
});

Deno.test("editing wins over a stale persisted collapsed preference", () => {
  assertEquals(
    pendingPanelDisclosureDecision({
      collapsed: true,
      editing: true,
      dirty: false,
    }),
    "discard-clean-edit-and-collapse",
  );
  assertEquals(
    pendingPanelDisclosureDecision({
      collapsed: true,
      editing: true,
      dirty: true,
    }),
    "confirm-dirty-edit",
  );
});

Deno.test("keyboard dismiss persists the live editor, not a stale empty mirror", () => {
  assertEquals(
    pendingEditLiveText("typed in the textarea", ""),
    "typed in the textarea",
  );
  assertEquals(pendingEditLiveText("", "stale mirror"), "");
  assertEquals(pendingEditLiveText(undefined, "mirrored"), "mirrored");
  assertEquals(
    pendingRowVisibleText("server still has the old draft", "just typed"),
    "just typed",
  );
  assertEquals(pendingRowVisibleText("server text", null), "server text");
});
