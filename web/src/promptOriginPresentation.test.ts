import { assertEquals } from "jsr:@std/assert";
import {
  agentOriginDisplayName,
  agentOriginSourceLabel,
  cowboyOriginCaption,
  originProviderId,
} from "./promptOriginPresentation";

Deno.test("Grok runtime notes use the short Grok name", () => {
  assertEquals(agentOriginDisplayName("grok"), "Grok");
  assertEquals(agentOriginDisplayName("claude-code"), "Claude Code");
  assertEquals(
    originProviderId({ actor: "agent", source: "runtime", provider: "grok" }, "codex"),
    "grok",
  );
  assertEquals(
    agentOriginSourceLabel({ actor: "agent", source: "runtime" }),
    "runtime",
  );
});

Deno.test("Cowboy notes keep the existing resume and schedule captions", () => {
  assertEquals(
    cowboyOriginCaption({ actor: "cowboy", source: "auto-resume" }),
    "Auto-resumed the interrupted turn",
  );
  assertEquals(
    cowboyOriginCaption({ actor: "cowboy", source: "schedule" }),
    "Scheduled wakeup",
  );
});
