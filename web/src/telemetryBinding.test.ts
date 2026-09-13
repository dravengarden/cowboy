import {
  assert,
  assertEquals,
  assertRejects,
  assertThrows,
} from "jsr:@std/assert";
import {
  confirmResolutionOnce,
  matchesResolution,
  parseBindingStatus,
  parseResolutionPlan,
  parseResolutionReceipt,
  PreviewDeadline,
  telemetryBindingApi,
} from "./telemetryBinding.ts";

const fixture = JSON.parse(
  Deno.readTextFileSync(
    new URL(
      "../../tests/fixtures/telemetry-resolution-surface.json",
      import.meta.url,
    ),
  ),
).public;
const signal = () => new AbortController().signal;
const response = (value: unknown) =>
  new Response(JSON.stringify(value), {
    headers: { "content-type": "application/json" },
  });

Deno.test("Rust public projections and closed Web decoders agree", () => {
  assertEquals(parseBindingStatus(fixture.absent), fixture.absent);
  assertEquals(parseBindingStatus(fixture.retained), fixture.retained);
  assertEquals(parseResolutionPlan(fixture.plan), fixture.plan);
  assertEquals(parseResolutionReceipt(fixture.receipt), fixture.receipt);
  assert(
    matchesResolution(
      parseResolutionPlan(fixture.plan),
      parseResolutionReceipt(fixture.receipt),
    ),
  );
  assert(!JSON.stringify(fixture).includes("operator"));
});

Deno.test("resolution schemas reject unknown actions, authority fields and incoherent phases", () => {
  for (
    const value of [
      null,
      {},
      { ...fixture.plan, force: true },
      { ...fixture.plan, actor: "injected" },
      { ...fixture.plan, schema: 2 },
      { ...fixture.plan, action: "repair" },
      { ...fixture.plan, result_phase: "completed" },
      { ...fixture.plan, expires_at_ms: Number.MAX_SAFE_INTEGER + 1 },
      { ...fixture.plan, confirmation_available: "true" },
      {
        ...fixture.plan,
        operation: { ...fixture.plan.operation, phase: "needs_attention" },
      },
      {
        ...fixture.plan,
        operation: { ...fixture.plan.operation, expected: {} },
      },
    ]
  ) {
    assertThrows(() => parseResolutionPlan(value));
  }
  for (
    const value of [
      { ...fixture.receipt, phase: "completed" },
      { ...fixture.receipt, endpoint: "private" },
      { ...fixture.receipt, resolved_at_ms: -1 },
      { ...fixture.receipt, operation_digest: "sha256:bad" },
    ]
  ) {
    assertThrows(() => parseResolutionReceipt(value));
  }
  assertThrows(() =>
    parseBindingStatus({
      ...fixture.absent,
      journal: { state: "absent", selection: null },
    })
  );
});

Deno.test("binding revisions remain canonical u64 decimal strings, never JS numbers", () => {
  const installation = {
    plugin_id: "victoria",
    plugin_version: "1.2.0",
    generation_digest: `sha256:${"ab".repeat(32)}`,
    installation_revision: `installation-${"cd".repeat(32)}`,
    contract_fingerprint: `sha256:${"ef".repeat(32)}`,
  };
  const current = {
    revision: "18446744073709551615",
    policy_epoch: "9007199254740993",
    selection: installation,
  };
  const value = {
    ...fixture.retained,
    journal: { ...fixture.retained.journal, current },
  };
  assertEquals(parseBindingStatus(value).journal, value.journal);
  for (const revision of [1, "01", "-1", "18446744073709551616", "1e3", "0"]) {
    assertThrows(() =>
      parseBindingStatus({
        ...value,
        journal: { ...value.journal, current: { ...current, revision } },
      })
    );
  }
  assertThrows(() =>
    parseBindingStatus({
      ...value,
      journal: {
        ...value.journal,
        current: {
          ...current,
          selection: { ...installation, token: "private" },
        },
      },
    })
  );
});

Deno.test("receipt correlation binds action, target, full original digest and new confirmation ID", () => {
  const plan = parseResolutionPlan(fixture.plan);
  const receipt = parseResolutionReceipt(fixture.receipt);
  for (
    const extra of [
      { resolution_id: "another-confirmation" },
      { operation_id: "another-operation" },
      { machine_id: "other-machine" },
      { operation_digest: `sha256:${"12".repeat(32)}` },
      { action: "accept_applied" as const, phase: "completed" as const },
    ]
  ) {
    assertEquals(matchesResolution(plan, { ...receipt, ...extra }), false);
  }
});

Deno.test("preview expiry is sticky across slow, regressed and repaired clocks", () => {
  const wall = 1_000_000;
  for (
    const [mono, nextWall] of [
      [120_100, wall + 1],
      [101, wall - 1],
      [99, wall],
      [101, wall + 120_000],
      [NaN, wall],
    ]
  ) {
    const deadline = new PreviewDeadline(wall + 120_000, 100, wall);
    assertEquals(deadline.ended(100, wall), false);
    assertEquals(deadline.ended(mono, nextWall), true);
    assertEquals(deadline.ended(101, wall + 1), true);
  }
  const deadline = new PreviewDeadline(wall + 10, 100, wall);
  assertEquals(deadline.ended(111, wall + 1), true);
});

Deno.test("HTTP confirmation sends only the finite plan reference with fresh noncached credentials", async () => {
  const previous = globalThis.fetch;
  const calls: { url: string; init?: RequestInit }[] = [];
  const plan = parseResolutionPlan(fixture.plan);
  const callSignal = signal();
  globalThis.fetch = ((url, init) => {
    calls.push({ url: String(url), init });
    return Promise.resolve(response(fixture.receipt));
  }) as typeof fetch;
  try {
    await telemetryBindingApi.confirm(plan, callSignal);
    assertEquals(calls.length, 1);
    assert(calls[0].url.endsWith("/resolve"));
    assertEquals(calls[0].init?.method, "POST");
    assertEquals(calls[0].init?.credentials, "same-origin");
    assertEquals(calls[0].init?.cache, "no-store");
    assertEquals(calls[0].init?.signal, callSignal);
    assertEquals(JSON.parse(String(calls[0].init?.body)), {
      plan_id: plan.plan_id,
      action: plan.action,
    });
  } finally {
    globalThis.fetch = previous;
  }
});

Deno.test("lost confirmation response permits one exact GET, never another POST", async () => {
  const previous = globalThis.fetch;
  const plan = parseResolutionPlan(fixture.plan);
  for (const outcome of ["exact", "foreign", "missing"]) {
    const methods: string[] = [];
    globalThis.fetch = ((_url, init) => {
      methods.push(init?.method ?? "GET");
      if (methods.length === 1) {
        return Promise.reject(new Error("lost HTTP acknowledgement"));
      }
      if (outcome === "missing") {
        return Promise.resolve(new Response("", { status: 404 }));
      }
      return Promise.resolve(
        response(
          outcome === "exact"
            ? fixture.receipt
            : { ...fixture.receipt, resolution_id: "foreign-resolution-id" },
        ),
      );
    }) as typeof fetch;
    try {
      assertEquals(
        await confirmResolutionOnce(plan, signal(), signal),
        outcome === "exact" ? fixture.receipt : null,
      );
      assertEquals(methods, ["POST", "GET"]);
    } finally {
      globalThis.fetch = previous;
    }
  }
});

Deno.test("ended owner forbids result inspection and successful confirmation needs no GET", async () => {
  const previous = globalThis.fetch;
  const plan = parseResolutionPlan(fixture.plan);
  for (const success of [true, false]) {
    let calls = 0;
    globalThis.fetch = (() => {
      calls++;
      return success
        ? Promise.resolve(response(fixture.receipt))
        : Promise.reject(new Error("observer ended"));
    }) as typeof fetch;
    try {
      const result = await confirmResolutionOnce(plan, signal(), () => {
        throw new Error("logout or unmount");
      });
      assertEquals(result, success ? fixture.receipt : null);
      assertEquals(calls, 1);
    } finally {
      globalThis.fetch = previous;
    }
  }
});

Deno.test("HTML, oversized, malformed and foreign responses never become successful evidence", async () => {
  const previous = globalThis.fetch;
  for (
    const make of [
      () => new Response("<html>old SPA</html>"),
      () => response("x".repeat(65_537)),
      () => response({ schema: 99 }),
      () =>
        response({ ...fixture.receipt, operation_id: "foreign-operation-id" }),
    ]
  ) {
    let calls = 0;
    globalThis.fetch = (() => {
      calls++;
      return Promise.resolve(make());
    }) as typeof fetch;
    try {
      await assertRejects(() =>
        telemetryBindingApi.receipt(
          fixture.plan.operation.operation_id,
          signal(),
        )
      );
      assertEquals(calls, 1);
    } finally {
      globalThis.fetch = previous;
    }
  }
});

Deno.test("the product Info entry uses core mobile/desktop confirmation and clears stale scopes", () => {
  const panel = Deno.readTextFileSync(
    new URL("./TelemetryBindingPanel.tsx", import.meta.url),
  );
  const info = Deno.readTextFileSync(
    new URL("./InfoSheet.tsx", import.meta.url),
  );
  assert(info.includes("<TelemetryBindingPanel />"));
  assert(panel.includes("<ConfirmSheet"));
  assert(!panel.includes("<Dialog"));
  assert(panel.includes("cowboy:product-sign-out"));
  assert(panel.includes("pending.current?.abort()"));
  assert(
    panel.includes("setPlan(null); // Never preserve a submitted confirmation"),
  );
  assert(panel.includes("plan?.confirmation_available"));
  assert(panel.includes("Already emitted telemetry cannot be undone"));
  assert(panel.includes("No binding head recorded on the Service"));
  assert(!panel.includes("No managed Machine namespace observed"));
});
