import { assertEquals } from "jsr:@std/assert";
import { machineCommandResultPresentation } from "./machineCommandResult.ts";

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("Machine command results never expose peer diagnostics", () => {
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

Deno.test("Machine command feedback is scoped to the current action and expires", () => {
  assertEquals(appSource.includes("commandFeedbackTimers"), true);
  assertEquals(appSource.includes("showCommandFeedback(machineId"), true);
  assertEquals(appSource.includes("}, 4_500);"), true);
  assertEquals(appSource.includes("events[machine.id]?.at(-1)"), false);
});
