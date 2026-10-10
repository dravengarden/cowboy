import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { machineCommandResultPresentation } from "./machineCommandResult.ts";

const appSource = await readFile(
  new URL("./App.tsx", import.meta.url), "utf8",
);

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
