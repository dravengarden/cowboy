import { test } from "bun:test";
import { assert, assertEquals, assertThrows } from "@std/assert";
import { canReplaceLogin, loginFailure, loginLink, loginOptions, subscriptionLogin, type LoginOptions } from "./subscriptionLoginController";
import { revocationProgress, validRevocationPending } from "./SubscriptionRevocation";
import { rememberSubscriptionSignIn, restoreSubscriptionSignIn, SUBSCRIPTION_SIGN_IN_RETURN, subscriptionSignInLink } from "./subscriptionSignInLink";

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

test("receipt polling retains the current challenge, while cancellation removes it immediately", async () => {
  const f = fixture();
  const views: { links: number; status: string | undefined }[] = [];
  f.setFetch(async (path, init) => Response.json(path.includes("/api/providers/") ? { request_id: "request", events: [{
    event: "login_challenge", provider: scope.provider, request_id: "request",
    verification_url: "https://claude.ai/oauth/authorize?state=private", expires_at_ms: Date.now() + 60_000,
  }] } : { status: init?.method === "DELETE" ? "cancellation_requested" : "active", cancel_requested: init?.method === "DELETE",
    request_id: "request", account_id: scope.account, provider_id: scope.provider, machine_id: "machine" }));
  const owner = subscriptionLogin(scope, { ...f.ports, changed: (view) => { views.push({ links: view.events.length, status: view.observation?.status }); } });
  owner.restore(); await owner.start(options, "native");
  views.length = 0;
  await owner.check();
  assert(views.length > 0);
  assert(views.every((view) => view.links === 1 && view.status === "active"));
  await owner.cancel();
  assertEquals(owner.snapshot.events.length, 0);
  assertEquals(owner.snapshot.observation?.cancel_requested, true);
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

test("agent login continuation restores only the original request and never restarts it", async () => {
  const f = fixture();
  const owner = subscriptionLogin(scope, f.ports); owner.restore();
  await owner.resume("request");
  assertEquals(owner.snapshot.pending?.request, "request");
  assertEquals(f.calls[0]!.path, "/api/provider-accounts/account/logins/request");
  assert(f.calls.every((call) => !call.init?.method));
  const restored = subscriptionLogin(scope, f.ports); restored.restore();
  const reads = f.calls.length;
  await restored.retry();
  await restored.start(options, "native");
  assertEquals(f.calls.length, reads);
  await restored.check();
  await restored.submit("one-time-code");
  const post = f.calls.filter((call) => call.init?.method === "POST");
  assertEquals(post.length, 1);
  assertEquals(post[0]!.path, "/api/providers/claude-code/auth/request");
  assert(![...f.values.values()].join().includes("one-time-code"));
  f.setFetch(async () => Response.json({ status: "cancellation_requested", request_id: "request" }));
  await restored.cancel();
  assertEquals(f.calls.at(-1)!.path, "/api/provider-accounts/account/logins/request");
  assertEquals(f.calls.at(-1)!.init?.method, "DELETE");
  assertEquals(restored.snapshot.pending?.cancelling, true);
});

test("continuation cannot adopt a wrong owner, account, request or an uncertain reference", async () => {
  for (const bad of [
    { status: "active", request_id: "request", account_id: "other", provider_id: scope.provider, machine_id: "machine" },
    { status: "active", request_id: "other", account_id: scope.account, provider_id: scope.provider, machine_id: "machine" },
    { status: "unknown", dispatch_reserved: false },
  ]) {
    const f = fixture(); f.setFetch(async () => Response.json(bad));
    const owner = subscriptionLogin(scope, f.ports); owner.restore(); await owner.resume("request");
    assert(owner.snapshot.error); assertEquals(f.values.size, 0);
    assert(!owner.snapshot.pending); assertEquals(owner.snapshot.events.length, 0);
  }
  const f = fixture();
  const owner = subscriptionLogin(scope, f.ports); owner.restore();
  f.setFetch(async () => { f.invalidate(); return Response.json({ status: "active", request_id: "request", account_id: scope.account, provider_id: scope.provider, machine_id: "machine" }); });
  await owner.resume("request"); assertEquals(f.values.size, 0);
});

test("continuation preserves another pending sign-in and refuses mixed dispatch metadata", async () => {
  const f = fixture();
  const owner = subscriptionLogin(scope, f.ports); owner.restore(); await owner.start(options, "native");
  const before = [...f.values.entries()]; const calls = f.calls.length;
  await owner.resume("other-request");
  assertEquals([...f.values.entries()], before); assertEquals(f.calls.length, calls);
  f.values.set(before[0]![0], JSON.stringify({ ...JSON.parse(before[0]![1]), request: "request" }));
  const invalid = subscriptionLogin(scope, f.ports); invalid.restore(); await invalid.resume("request");
  assert(invalid.snapshot.error); assertEquals(f.calls.length, calls);
});

test("a GUI operation can reopen its discovered request without redispatch", async () => {
  const f = fixture();
  const first = subscriptionLogin(scope, f.ports); first.restore(); await first.start(options, "native");
  const saved = [...f.values.entries()]; const writes = f.calls.filter((call) => call.init?.method === "POST").length;
  const next = subscriptionLogin(scope, f.ports); next.restore(); await next.resume("request");
  assertEquals(next.snapshot.error, ""); assertEquals(next.snapshot.observation?.request_id, "request");
  assertEquals([...f.values.entries()], saved);
  assertEquals(f.calls.filter((call) => call.init?.method === "POST").length, writes);
});

test("closing a durably cancelled attempt clears only the tab pointer and never retries", async () => {
  const f = fixture();
  const owner = subscriptionLogin(scope, f.ports); owner.restore(); await owner.start(options, "native");
  f.setFetch(async () => { throw new Error("cancellation uncertain"); });
  await owner.cancel(); owner.finish(); assert(owner.snapshot.pending);
  f.setFetch(async () => Response.json({ status: "cancellation_requested", cancel_requested: true, request_id: "request" }));
  await owner.check(); const count = f.calls.length;
  owner.finish(); assertEquals(f.values.size, 0); assertEquals(owner.snapshot.pending, undefined); assertEquals(f.calls.length, count);
});

test("an expired continuation prepares a fresh selection only after cancellation and never dispatches on restart", async () => {
  const f = fixture();
  f.setFetch(async (_path, init) => Response.json({
    status: init?.method === "DELETE" ? "cancellation_requested" : "inactive",
    cancel_requested: init?.method === "DELETE",
    request_id: "request", account_id: scope.account, provider_id: scope.provider, machine_id: "machine",
  }));
  const owner = subscriptionLogin(scope, f.ports); owner.restore(); await owner.resume("request");
  assert(canReplaceLogin(owner.snapshot));
  await owner.prepareNew();
  assertEquals(owner.snapshot.pending, undefined); assertEquals(f.values.size, 0);
  assertEquals(f.calls.filter((call) => call.init?.method).map((call) => [call.path, call.init?.method]),
    [["/api/provider-accounts/account/logins/request", "DELETE"]]);
});

test("unknown and active sign-ins cannot be silently replaced; failed cancellation retains the original request", async () => {
  for (const status of ["active", "unknown", "inactive"]) {
    const f = fixture();
    f.setFetch(async (path, init) => {
      if (init?.method === "DELETE") throw new Error("reply lost");
      return Response.json(path.includes("/api/providers/") ? { request_id: "request", events: [] } : {
        status, request_id: "request", account_id: scope.account, provider_id: scope.provider, machine_id: "machine",
      });
    });
    const owner = subscriptionLogin(scope, f.ports); owner.restore(); await owner.resume("request");
    const before = f.calls.length; await owner.prepareNew();
    assert(owner.snapshot.pending); assertEquals(owner.snapshot.pending.request, "request");
    assertEquals(f.calls.length, before + (status === "inactive" ? 1 : 0));
    assertEquals(f.calls.filter((call) => call.init?.method === "POST").length, 0);
    const restored = subscriptionLogin(scope, f.ports); restored.restore();
    assertEquals(restored.snapshot.pending?.request, "request");
  }
});

test("native failure is visible and permits only an explicitly cancelled replacement", async () => {
  for (const failure of [
    { event: "login_state", request_id: "request", provider: scope.provider, state: "error", detail: "Native profile cleanup is still pending" },
    { event: "command_result", request_id: "request", accepted: false, detail: "Native login is unavailable" },
  ]) {
    const f = fixture();
    f.setFetch(async (path, init) => Response.json(path.includes("/api/providers/")
      ? { request_id: "request", events: [failure] }
      : { status: init?.method === "DELETE" ? "cancellation_requested" : "active", cancel_requested: init?.method === "DELETE",
        request_id: "request", account_id: scope.account, provider_id: scope.provider, machine_id: "machine" }));
    const owner = subscriptionLogin(scope, f.ports); owner.restore(); await owner.resume("request");
    assertEquals(loginFailure(owner.snapshot), failure.detail);
    assert(canReplaceLogin(owner.snapshot));
    await owner.prepareNew();
    assert(!owner.snapshot.pending);
    assertEquals(f.calls.filter((call) => call.init?.method).map((call) => call.init?.method), ["DELETE"]);
  }
});

test("an unavailable saved operation is retryable without declaring a reference mismatch", async () => {
  const f = fixture();
  const original = subscriptionLogin(scope, f.ports); original.restore(); await original.start(options, "native");
  const saved = [...f.values.entries()];
  f.setFetch(async () => { throw new Error("offline"); });
  const owner = subscriptionLogin(scope, f.ports); owner.restore(); await owner.resume("request");
  assert(owner.snapshot.error); assert(!owner.snapshot.referenceMismatch);
  assertEquals([...f.values.entries()], saved);
  f.setFetch(async (path) => Response.json(path.includes("/api/providers/") ? { request_id: "request", events: [] }
    : { status: "active", request_id: "request", account_id: scope.account, provider_id: scope.provider, machine_id: "machine" }));
  await owner.resume("request");
  assertEquals(owner.snapshot.error, ""); assertEquals(owner.snapshot.observation?.request_id, "request");
  assertEquals(f.calls.filter((call) => call.init?.method === "POST").length, 1);
});

test("sign-in links contain only strict account and request references", () => {
  const account = "fd376b94-c493-4d27-81da-67a1d154ca17";
  const request = "provider-login-d1c970fe-2420-4d83-ab57-01ed5325a30e";
  const hash = `#subscription-sign-in/${account}/${request}`;
  assertEquals(subscriptionSignInLink(hash), { account, request });
  for (const invalid of [`${hash}?code=secret`, `${hash}/extra`, hash.replace(account, "default"), hash.replace(request, "../other"), "#sessions/abc"]) {
    assertEquals(subscriptionSignInLink(invalid), undefined);
  }
});

test("product sign-in redirects restore a reference once without overriding another route", () => {
  const f = fixture();
  const hash = "#subscription-sign-in/fd376b94-c493-4d27-81da-67a1d154ca17/provider-login-d1c970fe-2420-4d83-ab57-01ed5325a30e";
  let destination: string | URL | null | undefined;
  const navigation = { state: null, replaceState: (_state: unknown, _unused: string, url?: string | URL | null) => { destination = url; } };
  const current = { pathname: "/", search: "?surface=mobile", hash: "" };
  rememberSubscriptionSignIn(hash, f.ports.storage);
  assertEquals(f.values.get(SUBSCRIPTION_SIGN_IN_RETURN), hash);
  restoreSubscriptionSignIn(current, navigation, f.ports.storage);
  assertEquals(destination, `/?surface=mobile${hash}`);
  assertEquals(f.values.size, 0);
  destination = undefined;
  restoreSubscriptionSignIn(current, navigation, f.ports.storage);
  assertEquals(destination, undefined);
  for (const route of [{ ...current, hash: "#sessions/current" }, { ...current, pathname: "/auth/device" }]) {
    rememberSubscriptionSignIn(hash, f.ports.storage);
    restoreSubscriptionSignIn(route, navigation, f.ports.storage);
    assertEquals(destination, undefined); assertEquals(f.values.size, 0);
  }
  rememberSubscriptionSignIn(`${hash}?code=secret`, f.ports.storage);
  assertEquals(f.values.size, 0);
  f.values.set(SUBSCRIPTION_SIGN_IN_RETURN, "https://other.example");
  restoreSubscriptionSignIn(current, navigation, f.ports.storage);
  assertEquals(destination, undefined); assertEquals(f.values.size, 0);
  restoreSubscriptionSignIn(current, navigation, { getItem: () => { throw new Error("denied"); }, removeItem: () => {} });
  assertEquals(destination, undefined);
});
