import { assertEquals } from "jsr:@std/assert";
import { machineCommandResultPresentation } from "./machineCommandResult.ts";

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
