import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { createBootstrapPresentation } from "./bootstrapPresentation.ts";

type Frame = { type: string; value?: string };
test("initial and reconnect layouts publish one complete baseline", async () => {
  for (const initial of ["cached screen", "live screen"]) {
    const applied: Frame[] = [];
    let screen = initial;
    const receive = createBootstrapPresentation<Frame>((frame) => {
      applied.push(frame);
      if (frame.type === "sessions") screen = "new sessions";
    });
    receive({ type: "sessions" });
    await Promise.resolve(); // separate transport tasks may have paints between them
    assertEquals(screen, initial);
    receive({ type: "sync_patch", value: "folders" });
    receive({ type: "sync_patch", value: "order" });
    assertEquals(applied, []);
    receive({ type: "bootstrap_complete" });
    assertEquals(applied.map((frame) => frame.type), [
      "sessions", "sync_patch", "sync_patch", "bootstrap_complete",
    ]);
    assertEquals(screen, "new sessions");
    receive({ type: "sessions", value: "ordinary live change" });
    assertEquals(applied.at(-1)?.value, "ordinary live change");
  }
});

test("admission controls bypass the baseline and a failed socket drops its buffer", () => {
  const applied: Frame[] = [];
  const apply = (frame: Frame): void => { applied.push(frame); };
  const failed = createBootstrapPresentation(apply);
  failed({ type: "sessions", value: "incomplete" });
  failed({ type: "client_capacity" });
  failed({ type: "auth_session" });
  const replacement = createBootstrapPresentation(apply);
  replacement({ type: "sessions", value: "replacement" });
  replacement({ type: "bootstrap_complete" });
  assertEquals(applied.map((frame) => frame.value ?? frame.type), [
    "client_capacity", "auth_session", "replacement", "bootstrap_complete",
  ]);
});
