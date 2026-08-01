import { strict as assert } from "node:assert";
import { verticalWorkspaceRegion } from "./verticalWorkspaceRegion";

Deno.test("Ctrl-K moves Prompt focus to the top bar, skipping auxiliary panels", () => {
  for (const region of [
    "prompt.composer",
    "prompt.plan",
    "prompt.queued",
    "prompt.draft",
  ]) {
    assert.equal(verticalWorkspaceRegion("prompt", region, -1), "topbar.controls");
  }
});

Deno.test("Ctrl-J returns from the top bar to the pane's primary region", () => {
  assert.equal(
    verticalWorkspaceRegion("prompt", "topbar.controls", 1),
    "prompt.composer",
  );
  assert.equal(
    verticalWorkspaceRegion("sessions", "topbar.controls", 1),
    "sessions.list",
  );
  assert.equal(
    verticalWorkspaceRegion("conversation", "topbar.controls", 1),
    "conversation.transcript",
  );
});

Deno.test("vertical movement does not wrap past the workspace edges", () => {
  assert.equal(verticalWorkspaceRegion("prompt", "topbar.controls", -1), null);
  assert.equal(verticalWorkspaceRegion("prompt", "prompt.composer", 1), null);
});
