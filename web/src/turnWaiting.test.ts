import { assertEquals } from "jsr:@std/assert";
import {
  QUIET_BADGE_MIN,
  WAITING_ELAPSED_VISIBLE_SECONDS,
  hasOpenTool,
  isTurnActivityUpdate,
  quietMinutes,
  shouldShowQuietBadge,
  waitingActivityLabel,
} from "./turnWaiting.ts";

Deno.test("agent wait activity names the provider immediately", () => {
  assertEquals(waitingActivityLabel("Grok", 0), "Waiting for Grok…");
  assertEquals(
    waitingActivityLabel("Grok", WAITING_ELAPSED_VISIBLE_SECONDS - 1),
    "Waiting for Grok…",
  );
});

Deno.test("agent wait activity exposes elapsed seconds after a short silence", () => {
  assertEquals(
    waitingActivityLabel("Grok", WAITING_ELAPSED_VISIBLE_SECONDS),
    "Waiting for Grok · 5s",
  );
  assertEquals(waitingActivityLabel("Codex", 53), "Waiting for Codex · 53s");
});

Deno.test("Codex terminal output is turn activity, usage snapshots are not", () => {
  assertEquals(isTurnActivityUpdate("tool_call_update"), true);
  assertEquals(isTurnActivityUpdate("agent_thought_chunk"), true);
  assertEquals(isTurnActivityUpdate("usage_update"), false);
  assertEquals(isTurnActivityUpdate("session_info_update"), false);
});

Deno.test("an in-flight tool is not silence", () => {
  assertEquals(
    hasOpenTool([{ kind: "thought" }, { kind: "tool", status: "in_progress" }]),
    true,
  );
  assertEquals(
    hasOpenTool([{ kind: "tool", status: "completed" }, { kind: "thought" }]),
    false,
  );
});

Deno.test("quiet badge ignores idle time from before the current turn", () => {
  assertEquals(quietMinutes(10 * 60_000, 0), 10);
  assertEquals(shouldShowQuietBadge(true, QUIET_BADGE_MIN, false), true);
  assertEquals(shouldShowQuietBadge(true, QUIET_BADGE_MIN, true), false);
  assertEquals(shouldShowQuietBadge(false, 56, false), false);
  assertEquals(shouldShowQuietBadge(true, QUIET_BADGE_MIN - 1, false), false);
});

Deno.test("live store treats dropped Codex terminal deltas as turn activity", async () => {
  const storeSource = await Deno.readTextFile(new URL("./store.ts", import.meta.url));
  const transcriptSource = await Deno.readTextFile(
    new URL("./Transcript.tsx", import.meta.url),
  );
  assertEquals(storeSource.includes("isTurnActivityUpdate"), true);
  assertEquals(storeSource.includes("markSessionTurnActivity"), true);
  assertEquals(storeSource.includes("sessionTurnActivityAt"), true);
  assertEquals(transcriptSource.includes("sessionTurnActivityAt(sessionId)"), true);
  assertEquals(transcriptSource.includes("hasOpenTool(items)"), true);
  assertEquals(
    transcriptSource.includes("shouldShowQuietBadge(working, quietMin, inFlightTool)"),
    true,
  );
});
