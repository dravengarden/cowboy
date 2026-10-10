import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { machinePresencePresentation } from "./machinePresence.ts";

test("machine reconnect grace is distinct from true offline presence", () => {
  assertEquals(machinePresencePresentation("online"), {
    indicator: "running",
    label: "online",
  });
  assertEquals(machinePresencePresentation("reconnecting"), {
    indicator: "starting",
    label: "Reconnecting",
  });
  assertEquals(machinePresencePresentation("offline"), {
    indicator: "exited",
    label: "offline",
  });
});
