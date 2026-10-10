import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  type CallApproval,
  callApprovalReason,
  callApprovalTitle,
  currentCallApproval,
  receiveCallApproval,
} from "./callApproval";

function approval(overrides: Partial<CallApproval> = {}): CallApproval {
  return {
    schema: 1,
    caller: "claude-code",
    reason: "calls_disabled",
    requests: 1,
    agents: ["codex"],
    items: [{ agent: "codex", purpose: "review", summary: "aspect=security" }],
    ttl_ms: 15_000,
    ...overrides,
  };
}

test("a pushed approval lives until its ttl or its removal", () => {
  receiveCallApproval("s", approval(), 1_000);
  assertEquals(currentCallApproval("s", 15_999)?.requests, 1);
  assertEquals(currentCallApproval("s", 16_000), null);
  // Each resubmission refreshes it.
  receiveCallApproval("s", approval({ requests: 2 }), 10_000);
  assertEquals(currentCallApproval("s", 20_000)?.requests, 2);
  receiveCallApproval("s", null);
  assertEquals(currentCallApproval("s", 20_000), null);
  assertEquals(currentCallApproval("other", 20_000), null);
});

test("the prompt names the caller, the targets and why", () => {
  assertEquals(callApprovalTitle(approval()), "Claude wants to call Codex");
  assertEquals(
    callApprovalTitle(
      approval({ requests: 3, agents: ["claude-code", "codex"] }),
    ),
    "Claude wants to start 3 calls (Claude, Codex)",
  );
  assertEquals(
    callApprovalReason(approval()),
    "Agent calls are off for this session.",
  );
  assertEquals(
    callApprovalReason(approval({ reason: "policy_denied" })),
    "Codex is not an allowed call target for this session.",
  );
});
