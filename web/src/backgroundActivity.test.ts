import { assertEquals } from "jsr:@std/assert";
import {
  backgroundProviderUpdateLabel,
  backgroundTasksLabel,
  waitingOnBackground,
} from "./backgroundActivity.ts";

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

Deno.test("only an automatic Provider update reads as background maintenance", () => {
  const update = { from: "3.19.3", to: "3.19.4", automatic: true };
  assertEquals(
    backgroundProviderUpdateLabel("starting", update),
    "Updating in background: 3.19.3 → 3.19.4",
  );
  // An explicit Reload is awaited by the person who asked for it.
  assertEquals(backgroundProviderUpdateLabel("starting", { ...update, automatic: false }), null);
  assertEquals(backgroundProviderUpdateLabel("starting", undefined), null);
  assertEquals(backgroundProviderUpdateLabel("running", update), null);
});
