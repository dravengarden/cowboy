import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { machineCommandResultPresentation } from "./machineCommandResult.ts";
import { readAppSource } from "./appSource.test.ts";

const appSource = await readAppSource();

test("Machine command results never expose peer diagnostics", () => {
  assertEquals(machineCommandResultPresentation(true), {
    severity: "success",
    message: "Command accepted",
  });
  assertEquals(machineCommandResultPresentation(false), {
    severity: "warning",
    message:
      "The Machine rejected this command. Refresh its inventory and retry after updating the Machine.",
  });
});

test("Machine command feedback is scoped to the current action and expires", () => {
  assertEquals(appSource.includes("commandFeedbackTimers"), true);
  assertEquals(appSource.includes("showCommandFeedback(machineId"), true);
  assertEquals(appSource.includes("}, 4_500);"), true);
  assertEquals(appSource.includes("events[machine.id]?.at(-1)"), false);
});
