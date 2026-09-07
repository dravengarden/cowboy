import { assert, assertEquals } from "jsr:@std/assert";
import {
  AuthApiError,
  type DeviceAuthorizationInfo,
  type DeviceAuthorizationRequest,
} from "./authApi.ts";
import {
  captureDeviceAuthorizationFromLocation,
  clearDeviceAuthorization,
  DEVICE_AUTH_STORAGE_KEY,
  DeviceAuthorizationFlow,
  parseDeviceAuthorization,
  sameDeviceAuthorization,
  storedDeviceAuthorization,
} from "./deviceAuthorization.ts";
import { sessionCountdownLabel } from "./sessionSchedule.ts";

const request: DeviceAuthorizationRequest = {
  request_id: "request_abcdefghijklmnopqrstuvwxyz",
  approval_token: "approval_abcdefghijklmnopqrstuvwxyz",
};
const another = {
  ...request,
  request_id: "another_abcdefghijklmnopqrstuvwxyz",
};
const initialNow = 1_900_000_000_000;
const info: DeviceAuthorizationInfo = {
  request_id: request.request_id,
  name: "Release test client",
  fingerprint: "SHA256:test",
  expires_at_ms: initialNow + 300_000,
  status: "pending",
};

function storageFixture() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
    removeItem: (key: string) => {
      values.delete(key);
    },
  };
}

function fixture() {
  let now = initialNow;
  let approvals = 0;
  let denials = 0;
  const cleared: DeviceAuthorizationRequest[] = [];
  const dependencies = {
    inspect: (
      _request: DeviceAuthorizationRequest,
      _signal: AbortSignal,
    ): Promise<DeviceAuthorizationInfo> => Promise.resolve(info),
    approve: (_request: DeviceAuthorizationRequest): Promise<unknown> => {
      approvals++;
      return Promise.resolve({ ok: true });
    },
    deny: (_request: DeviceAuthorizationRequest): Promise<unknown> => {
      denials++;
      return Promise.resolve({ ok: true });
    },
    authorize: (operation: () => Promise<unknown>) => operation(),
    clear: (value: DeviceAuthorizationRequest) => {
      cleared.push(value);
    },
    now: () => now,
  };
  return {
    dependencies,
    cleared,
    flow: new DeviceAuthorizationFlow(request, dependencies),
    advance: (milliseconds: number) => {
      now += milliseconds;
    },
    approvals: () => approvals,
    denials: () => denials,
  };
}

Deno.test("missing device links never inspect or authorize", async () => {
  const test = fixture();
  test.dependencies.inspect = () => {
    throw new Error("must not inspect");
  };
  const flow = new DeviceAuthorizationFlow(null, test.dependencies);
  await flow.inspect();
  await flow.approve();
  assertEquals(flow.getSnapshot().phase, "missing");
  assertEquals(test.approvals(), 0);
});

Deno.test("expired server requests are terminal and never expose raw error text", async () => {
  const test = fixture();
  let inspections = 0;
  test.dependencies.inspect = () => {
    inspections++;
    return Promise.reject(new AuthApiError("unhelpful server text", 410));
  };
  await test.flow.inspect();
  await test.flow.inspect();
  await test.flow.approve();
  assertEquals(test.flow.getSnapshot().phase, "unavailable");
  assertEquals(test.flow.getSnapshot().error, null);
  assertEquals(test.cleared, [request]);
  assertEquals(inspections, 1);
  assertEquals(test.approvals(), 0);
});

Deno.test("transport and temporary HTTP failures preserve the link for explicit read-only retry", async () => {
  for (
    const reason of [
      new TypeError("offline"),
      ...[401, 403, 404, 429, 500, 503].map((status) =>
        new AuthApiError("private server details", status)
      ),
    ]
  ) {
    const test = fixture();
    let attempt = 0;
    test.dependencies.inspect = () =>
      ++attempt === 1 ? Promise.reject(reason) : Promise.resolve(info);
    await test.flow.inspect();
    assertEquals(test.flow.getSnapshot().phase, "loading");
    assert(test.flow.getSnapshot().error);
    assertEquals(
      test.flow.getSnapshot().error?.includes("private server details"),
      false,
    );
    assertEquals(test.cleared.length, 0);
    await test.flow.approve();
    assertEquals(test.approvals(), 0);
    await test.flow.inspect();
    assertEquals(test.flow.getSnapshot().phase, "pending");
    assertEquals(test.flow.getSnapshot().error, null);
    assertEquals(test.approvals(), 0);
  }
});

Deno.test("countdown uses the absolute deadline, including background-tab time", async () => {
  const test = fixture();
  await test.flow.inspect();
  assertEquals(
    sessionCountdownLabel(test.flow.getSnapshot().remainingMs),
    "5m 00s",
  );
  test.advance(100_100);
  test.flow.tick();
  assertEquals(
    sessionCountdownLabel(test.flow.getSnapshot().remainingMs),
    "3m 20s",
  );
  test.advance(199_900);
  test.flow.tick();
  assertEquals(test.flow.getSnapshot().phase, "expired");
  assertEquals(test.flow.getSnapshot().remainingMs, 0);
  await test.flow.approve();
  await test.flow.deny();
  assertEquals(test.approvals(), 0);
  assertEquals(test.denials(), 0);
  assertEquals(test.cleared.length, 1);
});

Deno.test("clicking after a suspended timer's deadline cannot approve", async () => {
  const test = fixture();
  await test.flow.inspect();
  test.advance(310_000);
  await test.flow.approve();
  assertEquals(test.flow.getSnapshot().phase, "expired");
  assertEquals(test.approvals(), 0);
});

Deno.test("requests already expired at inspection never expose pending actions", async () => {
  const test = fixture();
  test.advance(300_001);
  await test.flow.inspect();
  assertEquals(test.flow.getSnapshot().phase, "expired");
  assertEquals(test.cleared.length, 1);
});

Deno.test("approval rechecks expiry after recent-auth interaction", async () => {
  const test = fixture();
  await test.flow.inspect();
  test.dependencies.authorize = (operation) => {
    test.advance(300_001);
    return operation();
  };
  await test.flow.approve();
  assertEquals(test.flow.getSnapshot().phase, "unavailable");
  assertEquals(test.approvals(), 0);
});

Deno.test("cancelled recent authentication keeps the pending request and explains cancellation", async () => {
  const test = fixture();
  await test.flow.inspect();
  test.dependencies.authorize = () =>
    Promise.reject(new DOMException("Cancelled", "AbortError"));
  await test.flow.approve();
  assertEquals(test.flow.getSnapshot().phase, "pending");
  assert(test.flow.getSnapshot().error?.includes("cancelled"));
  assertEquals(test.flow.getSnapshot().busy, false);
  assertEquals(test.approvals(), 0);
  assertEquals(test.cleared.length, 0);
});

Deno.test("a successful in-flight approval wins over the elapsed local countdown", async () => {
  const test = fixture();
  const response = Promise.withResolvers<unknown>();
  let approvals = 0;
  test.dependencies.approve = () => {
    approvals++;
    return response.promise;
  };
  await test.flow.inspect();
  const approval = test.flow.approve();
  await test.flow.approve();
  await test.flow.deny();
  test.advance(300_001);
  test.flow.tick();
  assertEquals(test.flow.getSnapshot().busy, true);
  assertEquals(approvals, 1);
  response.resolve({ ok: true });
  await approval;
  assertEquals(test.flow.getSnapshot().phase, "approved");
  assertEquals(test.cleared.length, 1);
});

Deno.test("failed approval can be checked again without automatically approving", async () => {
  const test = fixture();
  test.dependencies.approve = () =>
    Promise.reject(new TypeError("connection reset"));
  await test.flow.inspect();
  await test.flow.approve();
  assertEquals(test.flow.getSnapshot().phase, "pending");
  assertEquals(test.flow.getSnapshot().busy, false);
  assertEquals(test.cleared.length, 0);
  await test.flow.inspect();
  assertEquals(test.flow.getSnapshot().phase, "pending");
  assertEquals(test.flow.getSnapshot().error, null);
});

Deno.test("completed or denied inspection remains terminal after the deadline", async () => {
  for (const status of ["approved", "denied"] as const) {
    const test = fixture();
    test.dependencies.inspect = () => Promise.resolve({ ...info, status });
    await test.flow.inspect();
    test.advance(500_000);
    test.flow.tick();
    await test.flow.approve();
    assertEquals(test.flow.getSnapshot().phase, status);
    assertEquals(test.approvals(), 0);
    assertEquals(test.cleared.length, 1);
  }
});

Deno.test("cleanup and StrictMode restart fence stale inspection failures", async () => {
  const test = fixture();
  const previous = Promise.withResolvers<DeviceAuthorizationInfo>();
  const signals: AbortSignal[] = [];
  test.dependencies.inspect = (_request, signal) => {
    signals.push(signal);
    return signals.length === 1 ? previous.promise : Promise.resolve(info);
  };
  const first = test.flow.inspect();
  test.flow.stop();
  await test.flow.inspect();
  previous.reject(new AuthApiError("old link unavailable", 410));
  await first;
  assertEquals(signals[0]?.aborted, true);
  assertEquals(signals[1]?.aborted, false);
  assertEquals(test.flow.getSnapshot().phase, "pending");
  assertEquals(test.cleared.length, 0);
});

Deno.test("late approval from a replaced link cannot clear its successor", async () => {
  const test = fixture();
  const pending = Promise.withResolvers<unknown>();
  test.dependencies.approve = () => pending.promise;
  await test.flow.inspect();
  const first = test.flow.approve();
  test.flow.stop();
  pending.resolve({ ok: true });
  await first;
  assertEquals(test.cleared.length, 0);
});

Deno.test("an inspection response for another request is never actionable", async () => {
  const test = fixture();
  test.dependencies.inspect = () =>
    Promise.resolve({ ...info, request_id: another.request_id });
  await test.flow.inspect();
  await test.flow.approve();
  assertEquals(test.flow.getSnapshot().phase, "loading");
  assert(test.flow.getSnapshot().error);
  assertEquals(test.approvals(), 0);
});

Deno.test("fresh same-tab links replace old capabilities and are removed from the URL", () => {
  const storage = storageFixture();
  const replaced: (string | URL | null | undefined)[] = [];
  const history = {
    replaceState: (
      _state: unknown,
      _unused: string,
      url?: string | URL | null,
    ) => {
      replaced.push(url);
    },
  };
  for (const value of [request, another]) {
    const location = {
      pathname: "/auth/device",
      hash: `#${new URLSearchParams({
        request_id: value.request_id,
        approval_token: value.approval_token,
      })}`,
    };
    assert(captureDeviceAuthorizationFromLocation(location, storage, history));
    assertEquals(storedDeviceAuthorization(storage), value);
  }
  clearDeviceAuthorization(request, storage);
  assertEquals(storedDeviceAuthorization(storage), another);
  assertEquals(replaced, ["/auth/device", "/auth/device"]);
  clearDeviceAuthorization(another, storage);
  assertEquals(storedDeviceAuthorization(storage), null);
});

Deno.test("invalid fresh links never fall back to a previous request", () => {
  const storage = storageFixture();
  storage.setItem(DEVICE_AUTH_STORAGE_KEY, JSON.stringify(request));
  captureDeviceAuthorizationFromLocation(
    { pathname: "/auth/device", hash: "#request_id=bad" },
    storage,
    { replaceState: () => {} },
  );
  assertEquals(storedDeviceAuthorization(storage), null);
  for (
    const value of [null, "text", [], { request_id: 42 }, {
      ...request,
      approval_token: "bad&injected",
    }]
  ) {
    assertEquals(parseDeviceAuthorization(value), null);
  }
  assert(sameDeviceAuthorization(request, { ...request }));
  assertEquals(sameDeviceAuthorization(request, another), false);
});
