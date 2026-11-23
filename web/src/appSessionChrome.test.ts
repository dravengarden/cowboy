import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import type { SessionMeta } from "./protocol.ts";
import { sameAppChromeSessions } from "./sessionIdentity.ts";
import { sameSessionQuotaInputs } from "./usageAccounts.ts";

function session(overrides: Partial<SessionMeta> = {}): SessionMeta {
  return {
    id: "sess-1",
    title: "Test session",
    provider: "codex",
    provider_account_id: "default",
    cwd: "/tmp/project",
    machine_id: "hawk",
    status: "busy",
    origin: "web",
    updated_at_ms: 1_000,
    context_used: 10_000,
    context_size: 200_000,
    ...overrides,
  } as SessionMeta;
}

test("sameAppChromeSessions ignores updated_at_ms and context usage ticks", () => {
  const before = [session({ id: "a" }), session({ id: "b", status: "running" })];
  const after = [
    session({ id: "a", updated_at_ms: 9_999, context_used: 55_000, context_size: 250_000 }),
    session({ id: "b", status: "running", updated_at_ms: 8_888 }),
  ];
  assertEquals(sameAppChromeSessions(before, after), true);
});

test("sameAppChromeSessions reacts to status, title, badge, order, or membership changes", () => {
  const base = [session({ id: "a" }), session({ id: "b" })];
  assertEquals(
    sameAppChromeSessions(base, [session({ id: "a", status: "running" }), session({ id: "b" })]),
    false,
  );
  assertEquals(
    sameAppChromeSessions(base, [session({ id: "a", title: "Renamed" }), session({ id: "b" })]),
    false,
  );
  assertEquals(sameAppChromeSessions(base, [session({ id: "b" }), session({ id: "a" })]), false);
  assertEquals(sameAppChromeSessions(base, [session({ id: "a" })]), false);
  assertEquals(
    sameAppChromeSessions(base, [session({ id: "a", background_tasks: 2 }), session({ id: "b" })]),
    false,
  );
});

test("sameSessionQuotaInputs ignores sessions without named-account quota changes", () => {
  const before = [session({ id: "a" }), session({ id: "b", provider_account_id: "acct-1" })];
  const after = [
    session({ id: "a", updated_at_ms: 9_999, context_used: 99_999 }),
    session({ id: "b", provider_account_id: "acct-1", updated_at_ms: 9_999 }),
  ];
  assertEquals(sameSessionQuotaInputs(before, after), true);
});
