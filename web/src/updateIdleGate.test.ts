import { assert, assertEquals } from "jsr:@std/assert";
import { EDITOR_ENGAGEMENT_MS, editorHoldsUpdate } from "./updateIdleGate.ts";

const typing = {
  focusedEditable: true,
  windowFocused: true,
  composing: false,
  sinceInputMs: 2_000,
};

Deno.test("an editor being typed in holds a client update", () => {
  assertEquals(editorHoldsUpdate(typing), true);
  assertEquals(editorHoldsUpdate({ ...typing, focusedEditable: false }), false);
});

Deno.test("a caret resting in the Desktop composer does not hold an update forever", () => {
  assertEquals(
    editorHoldsUpdate({ ...typing, sinceInputMs: EDITOR_ENGAGEMENT_MS }),
    false,
  );
  // The user switched to another window; the caret never left the composer.
  assertEquals(editorHoldsUpdate({ ...typing, windowFocused: false }), false);
});

Deno.test("an open IME composition holds an update however long it has been idle", () => {
  assertEquals(
    editorHoldsUpdate({
      ...typing,
      composing: true,
      windowFocused: false,
      sinceInputMs: 10 * EDITOR_ENGAGEMENT_MS,
    }),
    true,
  );
});

Deno.test("the store's update gate asks the editor engagement policy", async () => {
  const store = await Deno.readTextFile(new URL("./store.ts", import.meta.url));
  const gate = store.slice(store.indexOf("export function canApplyUpdateNow()"));
  assert(gate.slice(0, 1_400).includes("editorHoldsUpdate({"));
});
