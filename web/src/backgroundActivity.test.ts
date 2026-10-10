import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  backgroundProviderUpdateLabel,
  providerUpdateProgress,
  backgroundTasksLabel,
  waitingOnBackground,
} from "./backgroundActivity.ts";

test("an idle session still waiting on background work reads as active", () => {
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

test("only an automatic Provider update reads as background maintenance", () => {
  const update = { from: "3.19.3", to: "3.19.4", automatic: true, started_at_ms: 0 };
  assertEquals(
    backgroundProviderUpdateLabel("starting", update),
    "Updating in background: 3.19.3 → 3.19.4",
  );
  // An explicit Reload is awaited by the person who asked for it.
  assertEquals(backgroundProviderUpdateLabel("starting", { ...update, automatic: false }), null);
  assertEquals(backgroundProviderUpdateLabel("starting", undefined), null);
  assertEquals(backgroundProviderUpdateLabel("running", update), null);
});

test("background update progress rises monotonically and never completes", () => {
  const start = 1_000_000;
  assertEquals(providerUpdateProgress(start, start), 5);
  // A controller clock ahead of the client never reads as negative progress.
  assertEquals(providerUpdateProgress(start, start - 5_000), 5);
  assertEquals(providerUpdateProgress(start, start + 18_000), 63);
  let previous = 0;
  for (const seconds of [1, 5, 10, 18, 33, 50, 120, 3_600]) {
    const value = providerUpdateProgress(start, start + seconds * 1_000);
    assertEquals(value >= previous && value <= 97, true);
    previous = value;
  }
  assertEquals(previous, 97);
});
