import { assertEquals } from "jsr:@std/assert";
import {
  resolveCompactionAction,
  resolveSessionAction,
} from "./agentCommands.ts";
import type { AvailableCommand } from "./protocol.ts";

function commands(...names: string[]): AvailableCommand[] {
  return names.map((name) => ({ name, description: name }));
}

const compact = {
  aliases: ["compact", "compress", "summarize"],
  fallback_command: "compact",
};

Deno.test("Provider-declared compaction discovers an advertised alias", () => {
  assertEquals(resolveCompactionAction(compact, commands("compress"))?.command, "/compress");
  assertEquals(resolveCompactionAction(compact, commands("SUMMARIZE"))?.command, "/SUMMARIZE");
});

Deno.test("Provider-declared compaction has an explicit cold-start fallback", () => {
  assertEquals(resolveCompactionAction(compact, [])?.command, "/compact");
  assertEquals(resolveCompactionAction(undefined, []), null);
});

Deno.test("clear remains a Provider-independent Cowboy reset", () => {
  const action = resolveSessionAction("clear", "future-agent", []);
  assertEquals(action?.kind, "reset");
  assertEquals(action?.destructive, true);
});
