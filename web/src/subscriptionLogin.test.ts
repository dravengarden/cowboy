import { test } from "bun:test";
import { assert, assertEquals, assertThrows } from "@std/assert";
import { loginLink, loginOptions, subscriptionLogin, type LoginOptions } from "./subscriptionLoginController";
import { revocationProgress, validRevocationPending } from "./SubscriptionRevocation";

const scope = { owner: "owner", dataset: "dataset", account: "account", provider: "claude-code" };
const options: LoginOptions = { account_id: scope.account, provider_id: scope.provider, machine_id: "machine",
  available: true, reason: null, release: { provider_version: "1", generation_digest: "digest" },
  methods: [{ id: "native", available: true, reason: null }] };
function fixture() {
  const values = new Map<string, string>();
  const calls: { path: string; init?: RequestInit }[] = [];
  let current = true;
  let fetch = async (path: string, _init?: RequestInit) => Response.json(path.includes("/api/providers/")
    ? { request_id: "request", events: [] }
    : { status: "active", request_id: "request", account_id: scope.account, provider_id: scope.provider, machine_id: "machine" });
  const ports = {
    fetch: async (path: string, init?: RequestInit) => { calls.push({ path, init }); return await fetch(path, init); },
    storage: { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); }, removeItem: (key: string) => { values.delete(key); } },
    current: () => current, changed: () => {}, operation: () => "operation",
  };
  return { values, calls, ports, setFetch: (value: typeof fetch) => { fetch = value; }, invalidate: () => { current = false; } };
}

test("lost sign-in reply and panel remount retain the exact operation without automatically dispatching", async () => {
  const f = fixture();
  f.setFetch(async () => { assertEquals(f.values.size, 1); throw new Error("reply lost"); });
  const first = subscriptionLogin(scope, f.ports); first.restore();
  await first.start(options, "native");
  const body = f.calls[0]!.init!.body;
  assert(first.snapshot.pending); assert(first.snapshot.error);
  const retry = subscriptionLogin(scope, f.ports); retry.restore();
  assertEquals(f.calls.length, 1);
  await retry.retry();
  assertEquals(f.calls[1]!.init!.body, body);
  await retry.start({ ...options, machine_id: "other" }, "native");
  assertEquals(f.calls.length, 2);
});

test("concurrent clicks dispatch once and durable cancellation cannot be retried as login", async () => {
  const f = fixture();
  const owner = subscriptionLogin(scope, f.ports); owner.restore();
  await Promise.all([owner.start(options, "native"), owner.start(options, "native")]);
  assertEquals(f.calls.filter((call) => call.init?.method === "POST").length, 1);
  f.setFetch(async () => { throw new Error("cancel reply lost"); });
  await owner.cancel();
  const last = f.calls.length;
  const restored = subscriptionLogin(scope, f.ports); restored.restore();
  await restored.retry();
  assertEquals(f.calls.length, last);
  await restored.cancel();
  assertEquals(f.calls.at(-1)!.init?.method, "DELETE");
  assertEquals(f.calls.at(-1)!.path, "/api/provider-accounts/login-operations/operation");
});

test("storage failure, wrong dataset, and changed product account prevent login effects", async () => {
  const f = fixture();
  const owner = subscriptionLogin(scope, { ...f.ports, storage: { ...f.ports.storage, setItem: () => { throw new Error("storage denied"); } } });
  owner.restore(); await owner.start(options, "native"); assertEquals(f.calls.length, 0);
  const saved = subscriptionLogin(scope, f.ports); saved.restore(); await saved.start(options, "native");
  const count = f.calls.length;
  const changed = subscriptionLogin({ ...scope, dataset: "other" }, f.ports); changed.restore(); await changed.retry();
  assertEquals(f.calls.length, count);
  f.invalidate(); await saved.cancel(); assertEquals(f.calls.length, count);
});

test("challenges are ephemeral, scoped, and cannot produce a dangerous navigation", async () => {
  const f = fixture();
  f.setFetch(async (path) => Response.json(path.includes("/api/providers/") ? { request_id: "request", events: [{
    event: "login_challenge", provider: "claude-code", request_id: "request", verification_url: "https://claude.ai/oauth/authorize?state=private-link",
    input_required: true, expires_at_ms: Date.now() + 60_000,
  }] } : { status: "active", request_id: "request", account_id: scope.account, provider_id: scope.provider, machine_id: "machine" }));
  const owner = subscriptionLogin(scope, f.ports); owner.restore(); await owner.start(options, "native");
  assertEquals(owner.snapshot.events.length, 1);
  assert(![...f.values.values()].join().includes("private-link"));
  await owner.submit("private-code");
  assert(![...f.values.values()].join().includes("private-code"));
  for (const link of ["javascript:alert(1)", "http://claude.ai", "https://user:secret@claude.ai/"]) assertEquals(loginLink(link), undefined);
  assertEquals(loginLink("https://claude.ai/oauth/authorize"), "https://claude.ai/oauth/authorize");
  f.setFetch(async () => Response.json({ status: "completed", account_id: "other" }));
  await owner.check(); assert(owner.snapshot.error.includes("does not match"));
  owner.finish(); assert(owner.snapshot.pending);
});

test("login readiness and cleanup completion reject mismatched or incomplete evidence", () => {
  assertThrows(() => loginOptions({ ...options, machine_id: "other" }, scope, "machine"));
  assertThrows(() => loginOptions({ ...options, account_id: "other" }, scope, "machine"));
  assertThrows(() => loginOptions({ ...options, native_profile: {} }, scope, "machine"));
  assertThrows(() => revocationProgress({ status: "completed", account_id: "account" }, "account"));
  assertThrows(() => revocationProgress({ status: "completed", account_id: "account", completion: { account_completed: false, missing: [] } }, "account"));
  assertEquals(revocationProgress({ status: "completed", account_id: "account", completion: { account_completed: true, missing: [] } }, "account").status, "completed");
  const input = { operation: "operation", dataset: "dataset", expected_revision: 1 };
  assert(validRevocationPending(input, "dataset"));
  assert(!validRevocationPending({ ...input, token: "secret" }, "dataset"));
  assert(!validRevocationPending(input, "other"));
});
