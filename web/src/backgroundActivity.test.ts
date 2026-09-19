import { assertEquals } from "jsr:@std/assert";
import { backgroundTasksLabel, waitingOnBackground } from "./backgroundActivity.ts";

Deno.test("an idle session still waiting on background work reads as active", () => {
  assertEquals(waitingOnBackground("running", 2), true);
  assertEquals(waitingOnBackground("running", 0), false);
  assertEquals(waitingOnBackground("running", undefined), false);
  // Other states keep their own presentation.
  for (const status of ["busy", "starting", "exited", "crashed", "interrupted"] as const) {
    assertEquals(waitingOnBackground(status, 3), false);
  }
  assertEquals(backgroundTasksLabel(1), "Waiting on 1 background task…");
  assertEquals(backgroundTasksLabel(2), "Waiting on 2 background tasks…");
});
