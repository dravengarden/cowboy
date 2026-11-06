import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { sessionAttentionLabel } from "./sessionAttentionModel";
import type { SessionMeta } from "./protocol";

const session: SessionMeta = { id: "s", provider: "codex", cwd: "/tmp", title: "Test", status: "busy" };
test("attention is a request state, independent of busy, idle or read state", () => {
  assertEquals(sessionAttentionLabel(session), null);
  assertEquals(sessionAttentionLabel({ ...session, attention: { approvals: 2, questions: 0 } }), "Approval needed");
  assertEquals(sessionAttentionLabel({ ...session, status: "running", attention: { approvals: 0, questions: 1 } }), "Answer needed");
  assertEquals(sessionAttentionLabel({ ...session, attention: { approvals: 0, questions: 1 } }, true), "Approval & answer needed");
  assertEquals(sessionAttentionLabel(session, true), "Approval needed");
  assertEquals(sessionAttentionLabel({ ...session, attention: { approvals: 0, questions: 0, other: 1 } }), "Action needed");
});
test("closing and trashed sessions never offer decisions", () => {
  assertEquals(sessionAttentionLabel({ ...session, closing: true }, true), null);
  assertEquals(sessionAttentionLabel({ ...session, trashed_at_ms: 1, attention: { approvals: 1, questions: 1 } }), null);
});
