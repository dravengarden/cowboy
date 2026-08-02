import { assertEquals } from "jsr:@std/assert";
import { isPureTerminalOutputDelta } from "./protocol.ts";

Deno.test("pure terminal output deltas are transient transcript telemetry", () => {
  assertEquals(
    isPureTerminalOutputDelta({
      sessionUpdate: "tool_call_update",
      toolCallId: "exec-1",
      _meta: {
        terminal_output_delta: {
          terminal_id: "exec-1",
          data: "one more log line\n",
        },
      },
    }),
    true,
  );
  assertEquals(
    isPureTerminalOutputDelta({
      sessionUpdate: "tool_call_update",
      toolCallId: "exec-1",
      status: "completed",
      _meta: {
        terminal_output_delta: {
          terminal_id: "exec-1",
          data: "done\n",
        },
      },
    }),
    false,
  );
});
