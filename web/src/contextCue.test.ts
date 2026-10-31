import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { contextCue } from "./contextCue";
import type { SessionContext } from "./sessionContext";

const label = (provider: string, account: string) => `${provider}:${account}`;
const base: SessionContext = {
  revision: 1,
  applied_revision: 1,
  sources: [],
  next_agent: null,
  source_sessions: [],
  delivery: null,
  max_sources: 32,
};
const accepted = (session_id: string) => ({
  operation: "op",
  session_id,
  state: "accepted" as const,
});

test("ordinary or unresolved context shows no cue", () => {
  assertEquals(contextCue("s", undefined, label), null);
  assertEquals(contextCue("s", base, label), null);
  assertEquals(
    contextCue("s", {
      ...base,
      revision: 2,
      next_agent: {
        provider: "claude-code",
        provider_account_id: "mink",
        machine_id: "hawk",
        runtime_machine_id: null,
        cwd: "suger",
        preset: null,
      },
      delivery: { ...accepted("t"), state: "unknown" },
    }, label),
    null,
  );
});

test("a pending new agent names its subscription", () => {
  assertEquals(
    contextCue("s", {
      ...base,
      revision: 2,
      sources: ["s"],
      next_agent: {
        provider: "claude-code",
        provider_account_id: "mink",
        machine_id: "hawk",
        runtime_machine_id: null,
        cwd: "suger",
        preset: null,
      },
    }, label),
    { kind: "pending_agent", provider: "claude-code", label: "claude-code:mink" },
  );
  assertEquals(
    contextCue("s", { ...base, revision: 2, sources: ["a", "b"] }, label),
    { kind: "pending_links", count: 2 },
  );
});

test("source and destination link to each other after a handoff", () => {
  assertEquals(
    contextCue("s", { ...base, delivery: accepted("t") }, label)?.kind,
    "continued_in",
  );
  assertEquals(
    contextCue("t", {
      ...base,
      sources: ["s", "x", "gone"],
      delivery: accepted("t"),
      source_sessions: [
        { id: "s", available: true, title: "Fix PR" },
        { id: "x", available: true },
        { id: "gone", available: false },
      ],
    }, label),
    { kind: "continued_from", id: "s", title: "Fix PR", more: 1 },
  );
});
