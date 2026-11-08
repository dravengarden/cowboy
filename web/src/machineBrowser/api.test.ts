import { test } from "bun:test";
import {
  assert,
  assertEquals,
  assertNotEquals,
  assertStringIncludes,
} from "../../../tools/test-assert.ts";
import {
  MachineBrowserError,
  browserErrorText,
  browserReasonText,
  browserStateLabel,
  browserStreamUrl,
  browserViewport,
  createMachineBrowserApi,
  namesInstance,
  sizeChanged,
  validBrowserRead,
  validBrowserState,
} from "./api";

const account = "11111111-2222-4333-8444-555555555555";
const state = {
  instance: { account_id: account, instance_id: "b-0123456789abcdef01234567", incarnation: 2 },
  state: "detached",
  geometry: { width: 1280, height: 800 },
  scale: 1,
  deadline_ms: 1_700_000_000_000,
  profile_revision: 3,
  unsaved_state_lost: false,
  observed_at_ms: 1_699_999_999_000,
};

test("admission and refusal codes never reach people raw", () => {
  const codes = [
    "machine_browser_not_enabled",
    "browser_not_enabled",
    "account_unavailable",
    "machine_unavailable",
    "browser_host_requirements",
    "browser_not_running",
    "browser_attached",
    "browser_open_failed",
    "browser_command_failed",
    "dataset_changed",
    "browser_attach_refused",
    "browser_attach_failed",
    "login_inactive",
    "login_mismatch",
    "browser_sign_in_refused",
    "invalid_viewport",
    "resize_failed",
  ];
  const fallback = browserErrorText("future_code");
  for (const code of codes) {
    const text = browserErrorText(code);
    assertEquals(text.includes("_"), false, code);
    assertNotEquals(text, fallback, code);
  }
  assertStringIncludes(browserErrorText("http_502"), "502");
  assertEquals(browserErrorText(undefined), fallback);
});

test("lifecycle reasons have text and absent reasons have none", () => {
  const reasons = [
    "not_available",
    "host_requirements",
    "sandbox_unavailable",
    "confinement_unverified",
    "listener_detected",
    "storage_exceeded",
    "profile_newer_than_runtime",
    "superseded",
    "login_ended",
    "login_mismatch",
    "deadline",
    "grace",
    "reclaim",
    "revoked",
    "disabled",
    "owner_request",
    "capacity",
    "memory",
    "busy",
    "failed",
  ];
  const unknown = browserReasonText("something_newer");
  assert(unknown !== undefined);
  for (const reason of reasons) {
    const text = browserReasonText(reason);
    assert(text !== undefined, reason);
    assertEquals(text.includes("_"), false, reason);
    assertNotEquals(text, unknown, reason);
  }
  assertEquals(browserReasonText(undefined), undefined);
  assertEquals(browserReasonText(null), undefined);
  assertEquals(browserStateLabel("detached"), "Running, not attached");
  assertEquals(browserStateLabel(null), "Never opened");
});

test("viewports scale the CSS size and stay within the wire bounds", () => {
  assertEquals(browserViewport(390, 700, 2), { width: 780, height: 1400, scale: 2 });
  assertEquals(browserViewport(100, 100, 1), { width: 320, height: 480, scale: 1 });
  assertEquals(browserViewport(5000, 3000, 2), { width: 4096, height: 4096, scale: 2 });
  assertEquals(browserViewport(Number.NaN, Number.POSITIVE_INFINITY, 1), {
    width: 320,
    height: 480,
    scale: 1,
  });
  assertEquals(browserViewport(1280.4, 799.6, 1), { width: 1280, height: 800, scale: 1 });
  assertEquals(sizeChanged({ width: 800, height: 600 }, { width: 815, height: 600 }, 16), false);
  assertEquals(sizeChanged({ width: 800, height: 600 }, { width: 816, height: 600 }, 16), true);
  assertEquals(sizeChanged({ width: 800, height: 600 }, { width: 800, height: 584 }, 16), true);
});

test("the stream URL names the shown instance, the viewport and only an explicit takeover", () => {
  const origin = { protocol: "https:", host: "cowboy.test" };
  const target = { machine_id: "ovh", instance: state.instance };
  const url = new URL(
    browserStreamUrl(account, target, { width: 780, height: 1400, scale: 2 }, false, origin),
  );
  assertEquals(url.searchParams.get("machine_id"), "ovh");
  assertEquals(url.searchParams.get("instance_id"), state.instance.instance_id);
  assertEquals(url.searchParams.get("incarnation"), String(state.instance.incarnation));
  assertEquals(url.protocol, "wss:");
  assertEquals(url.host, "cowboy.test");
  assertEquals(url.pathname, `/api/provider-accounts/${account}/browser/stream`);
  assertEquals(url.searchParams.get("width"), "780");
  assertEquals(url.searchParams.get("height"), "1400");
  assertEquals(url.searchParams.get("scale"), "2");
  assertEquals(url.searchParams.has("takeover"), false);
  const takeover = new URL(
    browserStreamUrl(account, target, { width: 1280, height: 800, scale: 1 }, true, {
      protocol: "http:",
      host: "localhost:5173",
    }),
  );
  assertEquals(takeover.protocol, "ws:");
  assertEquals(takeover.searchParams.get("takeover"), "true");
});

test("state reads are validated field by field", () => {
  assertEquals(validBrowserState(state), true);
  assertEquals(validBrowserState({ ...state, state: "sleeping" }), false);
  assertEquals(
    validBrowserState({ ...state, instance: { ...state.instance, incarnation: -1 } }),
    false,
  );
  assertEquals(validBrowserState({ ...state, geometry: { width: "wide", height: 1 } }), false);
  assertEquals(validBrowserState({ ...state, reason: "grace" }), true);
  const read = { machine_id: "ovh", state, placements: [], admission: null };
  assertEquals(validBrowserRead(read), true);
  assertEquals(validBrowserRead({ ...read, machine_id: null, state: null }), true);
  assertEquals(validBrowserRead({ ...read, admission: "browser_not_enabled" }), true);
  assertEquals(
    validBrowserRead({
      ...read,
      placements: [
        {
          account_id: account,
          machine_id: "ovh",
          instance_id: state.instance.instance_id,
          profile_revision: 1,
          last_open_ms: 1,
        },
      ],
    }),
    true,
  );
  assertEquals(validBrowserRead({ ...read, placements: [{ machine_id: "ovh" }] }), false);
  assertEquals(validBrowserRead({ ...read, admission: 7 }), false);
  assertEquals(validBrowserRead(null), false);
});

test("the Machine's no-instance answer is accepted only as hibernated", () => {
  const none = { account_id: account, instance_id: "", incarnation: 0 };
  const stopped = {
    instance: none,
    state: "hibernated",
    snapshot_generation: 4,
    profile_revision: 3,
    observed_at_ms: 1,
  };
  assertEquals(validBrowserState(stopped), true);
  assertEquals(namesInstance(stopped as Parameters<typeof namesInstance>[0]), false);
  assertEquals(namesInstance(state as Parameters<typeof namesInstance>[0]), true);
  // Every reset answers this shape, with or without a reason.
  assertEquals(validBrowserState({ ...stopped, reason: "owner_request" }), true);
  // A query while an unconfirmed unit of unknown owner exists (or the
  // Machine's inventory is pending) answers `stopping`/`busy` the same way.
  const unknown = { ...stopped, state: "stopping", reason: "busy" };
  assertEquals(validBrowserState(unknown), true);
  assertEquals(namesInstance(unknown as Parameters<typeof namesInstance>[0]), false);
  // Instance-scoped states keep strict identities.
  for (const running of ["starting", "detached", "attached"]) {
    assertEquals(validBrowserState({ ...stopped, state: running }), false, running);
  }
  assertEquals(validBrowserState({ ...unknown, instance: { ...none, incarnation: 2 } }), false);
  assertEquals(validBrowserState({ ...stopped, instance: { ...none, incarnation: 2 } }), false);
  assertEquals(
    validBrowserState({ ...state, instance: { ...state.instance, incarnation: 0 } }),
    true,
  );
  assertEquals(
    validBrowserRead({ machine_id: "ovh", state: stopped, placements: [], admission: null }),
    true,
  );
});

test("a reset answered without an instance is a valid result", async () => {
  const reply = {
    machine_id: "ovh",
    state: {
      instance: { account_id: account, instance_id: "", incarnation: 0 },
      state: "hibernated",
      reason: "owner_request",
      profile_revision: 4,
      observed_at_ms: 1,
    },
  };
  const api = createMachineBrowserApi(fakeFetch(() => ({ status: 200, body: reply })).fetch);
  assertEquals(
    await api.reset(account, "dataset-1", "ovh", 3, new AbortController().signal),
    reply,
  );
});

interface Call {
  path: string;
  init: RequestInit;
}

function fakeFetch(
  answer: (call: Call) => { status: number; body: unknown } | { status: number; text: string },
): { calls: Call[]; fetch: (input: string, init: RequestInit) => Promise<Response> } {
  const calls: Call[] = [];
  return {
    calls,
    fetch: (path, init) => {
      const call = { path, init };
      calls.push(call);
      const reply = answer(call);
      const text = "text" in reply ? reply.text : JSON.stringify(reply.body);
      return Promise.resolve(new Response(text, { status: reply.status }));
    },
  };
}

test("lifecycle calls hit their routes with the dataset and exact wire bodies", async () => {
  const machine = { machine_id: "ovh", state };
  const { calls, fetch } = fakeFetch(() => ({ status: 200, body: machine }));
  const api = createMachineBrowserApi(fetch);
  const signal = new AbortController().signal;
  const viewport = { width: 1280, height: 800, scale: 1 as const };
  assertEquals(await api.open(account, "dataset-1", viewport, signal), machine);
  assertEquals(
    await api.openSignIn(account, "provider-login-1", "dataset-1", viewport, signal),
    machine,
  );
  const target = { machine_id: "hawk", instance: { ...state.instance, extra: "dropped" } };
  await api.hibernate(account, "dataset-1", target, signal);
  await api.snapshot(account, "dataset-1", target, signal);
  await api.reset(account, "dataset-1", "hawk", 3, signal);
  const read = { machine_id: "ovh", state, placements: [], admission: null };
  const reads = fakeFetch(() => ({ status: 200, body: read }));
  const reader = createMachineBrowserApi(reads.fetch);
  assertEquals(await reader.read(account, signal), read);
  await reader.read(account, signal, "hawk");
  assertEquals(reads.calls[0]?.path, `/api/provider-accounts/${account}/browser`);
  assertEquals(reads.calls[1]?.path, `/api/provider-accounts/${account}/browser?machine_id=hawk`);
  assertEquals(reads.calls[0]?.init.method, "GET");
  assertEquals(reads.calls[0]?.init.cache, "no-store");
  const paths = calls.map((call) => call.path);
  assertEquals(paths, [
    `/api/provider-accounts/${account}/browser/open`,
    `/api/provider-accounts/${account}/logins/provider-login-1/browser`,
    `/api/provider-accounts/${account}/browser/hibernate`,
    `/api/provider-accounts/${account}/browser/snapshot`,
    `/api/provider-accounts/${account}/browser/reset`,
  ]);
  for (const call of calls) assertEquals(call.init.method, "POST");
  const bodies = calls.map((call) => JSON.parse(String(call.init.body)) as unknown);
  assertEquals(bodies[0], { dataset: "dataset-1", width: 1280, height: 800, scale: 1 });
  assertEquals(bodies[1], { dataset: "dataset-1", width: 1280, height: 800, scale: 1 });
  // Every instance-scoped request names the Machine shown with the instance.
  assertEquals(bodies[2], { dataset: "dataset-1", machine_id: "hawk", instance: state.instance });
  assertEquals(bodies[3], { dataset: "dataset-1", machine_id: "hawk", instance: state.instance });
  assertEquals(bodies[4], {
    dataset: "dataset-1",
    machine_id: "hawk",
    expected_profile_revision: 3,
  });
});

test("refusals surface their code and HTTP-only failures their status", async () => {
  const signal = new AbortController().signal;
  const refused = createMachineBrowserApi(
    fakeFetch(() => ({ status: 409, body: { error: { code: "browser_attached" } } })).fetch,
  );
  const error = await refused
    .open(account, "dataset-1", { width: 1280, height: 800, scale: 1 }, signal)
    .then(
      () => undefined,
      (cause: unknown) => cause,
    );
  assert(error instanceof MachineBrowserError);
  assertEquals(error.code, "browser_attached");
  assertEquals(error.status, 409);
  assertEquals(error.message, browserErrorText("browser_attached"));
  const html = createMachineBrowserApi(fakeFetch(() => ({ status: 502, text: "<html>" })).fetch);
  const gateway = await html.read(account, signal).then(
    () => undefined,
    (cause: unknown) => cause,
  );
  assert(gateway instanceof MachineBrowserError);
  assertEquals(gateway.code, "http_502");
  const odd = createMachineBrowserApi(fakeFetch(() => ({ status: 200, body: { state: 1 } })).fetch);
  const shape = await odd.read(account, signal).then(
    () => undefined,
    (cause: unknown) => cause,
  );
  assert(shape instanceof MachineBrowserError);
  assertEquals(shape.code, "invalid_response");
});
