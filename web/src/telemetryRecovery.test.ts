import {
  assert,
  assertEquals,
  assertRejects,
  assertThrows,
} from "jsr:@std/assert";
import { PreviewDeadline } from "./telemetryBinding.ts";
import {
  confirmRecoveryOnce,
  matchesRecovery,
  parseRecoveryAudit,
  parseRecoveryPlan,
  parseRecoveryReceipt,
  telemetryRecoveryApi,
} from "./telemetryRecovery.ts";

const fixture = JSON.parse(
  Deno.readTextFileSync(
    new URL(
      "../../tests/fixtures/telemetry-recovery-surface.json",
      import.meta.url,
    ),
  ),
);
const signal = () => new AbortController().signal;
const response = (value: unknown) =>
  new Response(JSON.stringify(value), {
    headers: { "content-type": "application/json" },
  });

Deno.test("Machine recovery projections agree with Rust and carry no serialized authority", () => {
  assertEquals(parseRecoveryPlan(fixture.plan), fixture.plan);
  assertEquals(parseRecoveryReceipt(fixture.receipt), fixture.receipt);
  assertEquals(parseRecoveryAudit(fixture.audit), fixture.audit);
  assert(
    matchesRecovery(
      parseRecoveryPlan(fixture.plan),
      parseRecoveryReceipt(fixture.receipt),
    ),
  );
  for (
    const field of ["actor", "endpoint", "observation_digest", "token", "step"]
  ) {
    assert(!JSON.stringify(fixture).includes(field));
  }
});

Deno.test("durable recovery audit binds the original before separately from a terminal Service operation", () => {
  const resolved = {
    ...fixture.audit,
    operation: {
      ...fixture.audit.operation,
      phase: "rejected",
      attention: null,
      operation_digest: `sha256:${"cd".repeat(32)}`,
    },
  };
  assertEquals(parseRecoveryAudit(resolved), resolved);
  assertEquals(
    parseRecoveryAudit({ ...resolved, recovery: null }).recovery,
    null,
  );
  for (
    const change of [
      { schema: 2 },
      { actor: "injected" },
      { recovery: undefined },
      { recovery: { ...resolved.recovery, grant: "injected" } },
      { recovery: { ...resolved.recovery, before: resolved.operation } },
      { operation: { ...resolved.operation, phase: "completed" } },
      { operation: { ...resolved.operation, machine_id: "another-machine" } },
      {
        operation: {
          ...fixture.audit.operation,
          operation_digest: `sha256:${"ef".repeat(32)}`,
        },
      },
      {
        recovery: {
          ...resolved.recovery,
          receipt: {
            ...fixture.receipt,
            operation_digest: resolved.operation.operation_digest,
          },
        },
      },
      {
        recovery: {
          ...resolved.recovery,
          receipt: { ...fixture.receipt, operation_id: "different-operation" },
        },
      },
    ]
  ) assertThrows(() => parseRecoveryAudit({ ...resolved, ...change }));
});

Deno.test("durable audit discovery sends one GET with no stored plan and rejects changed operation or unsupported Machine", async () => {
  const previous = globalThis.fetch;
  for (
    const mode of ["recorded", "absent", "changed", "unavailable", "oversized"]
  ) {
    let calls = 0;
    globalThis.fetch = ((url, init) => {
      calls++;
      assert(
        String(url).endsWith(
          `/${fixture.audit.operation.operation_id}/machine-recovery-audit`,
        ),
      );
      assertEquals(init?.method, "GET");
      assertEquals(init?.body, undefined);
      if (mode === "unavailable") {
        return Promise.resolve(new Response("{}", { status: 503 }));
      }
      return Promise.resolve(
        response(
          mode === "oversized" ? "x".repeat(65_537) : {
            ...fixture.audit,
            recovery: mode === "absent" || mode === "changed"
              ? null
              : fixture.audit.recovery,
            operation: mode === "changed"
              ? {
                ...fixture.audit.operation,
                operation_digest: `sha256:${"ab".repeat(32)}`,
              }
              : fixture.audit.operation,
          },
        ),
      );
    }) as typeof fetch;
    try {
      if (mode === "recorded" || mode === "absent") {
        assertEquals(
          (await telemetryRecoveryApi.audit(fixture.audit.operation, signal()))
            .recovery,
          mode === "absent" ? null : fixture.audit.recovery,
        );
      } else {
        await assertRejects(() =>
          telemetryRecoveryApi.audit(fixture.audit.operation, signal())
        );
      }
      assertEquals(calls, 1);
    } finally {
      globalThis.fetch = previous;
    }
  }
});

Deno.test("recovery decoders reject Service actions, changed heads, raw evidence and invalid counters", () => {
  for (
    const change of [
      { action: "record_rejected" },
      { action: "force" },
      { request_digest: "invalid" },
      { actor: "injected" },
      { schema: 2 },
      { confirmation_available: 1 },
      {
        operation: {
          ...fixture.plan.operation,
          phase: "prepared",
          attention: null,
        },
      },
      { operation: { ...fixture.plan.operation, attention: null } },
      { machine_head: { revision: "01", policy_epoch: "0", selection: null } },
      { machine_head: { revision: "1", policy_epoch: "0", selection: null } },
      {
        machine_head: {
          revision: "18446744073709551616",
          policy_epoch: "0",
          selection: null,
        },
      },
    ]
  ) assertThrows(() => parseRecoveryPlan({ ...fixture.plan, ...change }));
  for (
    const change of [
      { action: "record_rejected" },
      { phase: "rejected" },
      { resolved_at_ms: -1 },
      { request_digest: "bad" },
      { token: "private" },
    ]
  ) {
    assertThrows(() => parseRecoveryReceipt({ ...fixture.receipt, ...change }));
  }
  const head = {
    revision: "9007199254740993",
    policy_epoch: "9007199254740994",
    selection: null,
  };
  assertEquals(
    parseRecoveryPlan({
      ...fixture.plan,
      operation: { ...fixture.plan.operation, expected: head },
      machine_head: head,
    }).machine_head,
    head,
  );
});

Deno.test("recovery audit correlation includes full request and its original expiry", () => {
  const plan = parseRecoveryPlan(fixture.plan);
  const receipt = parseRecoveryReceipt(fixture.receipt);
  for (
    const change of [
      { resolution_id: "other-resolution" },
      { operation_id: "other-operation" },
      { machine_id: "other-machine" },
      { operation_digest: `sha256:${"ab".repeat(32)}` },
      { request_digest: `sha256:${"ab".repeat(32)}` },
      { resolved_at_ms: plan.expires_at_ms },
    ]
  ) assert(!matchesRecovery(plan, { ...receipt, ...change }));
  const deadline = new PreviewDeadline(1_500_000, 100, 1_000_000, 60_000);
  assert(!deadline.ended(100, 1_000_000));
  assert(deadline.ended(60_100, 1_000_001));
  assert(deadline.ended(101, 1_000_002));
});

Deno.test("Machine confirmation sends only a finite reference and never chains Service resolution", async () => {
  const previous = globalThis.fetch;
  const calls: { url: string; init?: RequestInit }[] = [];
  globalThis.fetch = ((url, init) => {
    calls.push({ url: String(url), init });
    return Promise.resolve(response(fixture.receipt));
  }) as typeof fetch;
  try {
    const plan = parseRecoveryPlan(fixture.plan);
    const result = await confirmRecoveryOnce(plan, signal(), signal);
    assertEquals(result, fixture.receipt);
    assertEquals(calls.length, 1);
    assert(calls[0].url.endsWith("/recover-machine"));
    assertEquals(calls[0].init?.method, "POST");
    assertEquals(calls[0].init?.cache, "no-store");
    assertEquals(calls[0].init?.credentials, "same-origin");
    assertEquals(JSON.parse(String(calls[0].init?.body)), {
      plan_id: plan.plan_id,
      action: plan.action,
    });
  } finally {
    globalThis.fetch = previous;
  }
});

Deno.test("lost HTTP response permits one exact Machine audit GET, not POST replay", async () => {
  const previous = globalThis.fetch;
  const plan = parseRecoveryPlan(fixture.plan);
  for (const outcome of ["exact", "foreign", "restart", "oversized", "html"]) {
    const methods: string[] = [];
    globalThis.fetch = ((url, init) => {
      methods.push(init?.method ?? "GET");
      if (methods.length === 1) {
        return Promise.reject(new Error("lost acknowledgement"));
      }
      assert(String(url).endsWith(`/machine-recoveries/${plan.plan_id}`));
      return Promise.resolve(
        outcome === "restart"
          ? new Response("", { status: 404 })
          : outcome === "html"
          ? new Response("<html>old Controller</html>")
          : response(
            outcome === "exact"
              ? fixture.receipt
              : outcome === "oversized"
              ? "x".repeat(65_537)
              : {
                ...fixture.receipt,
                request_digest: `sha256:${"de".repeat(32)}`,
              },
          ),
      );
    }) as typeof fetch;
    try {
      assertEquals(
        await confirmRecoveryOnce(plan, signal(), signal),
        outcome === "exact" ? fixture.receipt : null,
      );
      assertEquals(methods, ["POST", "GET"]);
    } finally {
      globalThis.fetch = previous;
    }
  }
});

Deno.test("logout or unmount refuses a new recovery inspection scope", async () => {
  const previous = globalThis.fetch;
  let calls = 0;
  globalThis.fetch = (() => {
    calls++;
    return Promise.reject(new Error("observer ended"));
  }) as typeof fetch;
  try {
    assertEquals(
      await confirmRecoveryOnce(
        parseRecoveryPlan(fixture.plan),
        signal(),
        () => {
          throw new Error("scope ended");
        },
      ),
      null,
    );
    assertEquals(calls, 1);
  } finally {
    globalThis.fetch = previous;
  }
});

Deno.test("preview requests reject another operation without retry", async () => {
  const previous = globalThis.fetch;
  let calls = 0;
  globalThis.fetch = ((_url, init) => {
    calls++;
    assertEquals(init?.body, "{}");
    return Promise.resolve(response(fixture.plan));
  }) as typeof fetch;
  try {
    await assertRejects(() =>
      telemetryRecoveryApi.plan("different-operation", signal())
    );
    assertEquals(calls, 1);
  } finally {
    globalThis.fetch = previous;
  }
});

Deno.test("Machine recovery uses a separate core ConfirmSheet under an exact operation scope", () => {
  const panel = Deno.readTextFileSync(
    new URL("./TelemetryRecoveryPanel.tsx", import.meta.url),
  );
  const parent = Deno.readTextFileSync(
    new URL("./TelemetryBindingPanel.tsx", import.meta.url),
  );
  assert(parent.includes("operation.operation_digest}"));
  assert(parent.includes("{operation && !busy && ("));
  assert(panel.includes('operation.phase === "needs_attention"'));
  assert(panel.includes("Inspect recorded Machine recovery"));
  assert(panel.includes("telemetryRecoveryApi.audit(operation, work.signal)"));
  assert(panel.includes("<ConfirmSheet"));
  assert(!panel.includes("<Dialog"));
  assert(panel.includes("cowboy:product-sign-out"));
  assert(panel.includes("pending.current?.abort()"));
  assert(panel.includes("submitted.current = value.plan_id"));
  assert(panel.includes("plan?.confirmation_available"));
  assert(panel.replace(/\s+/g, " ").includes("separate fresh confirmation"));
  assert(!panel.includes("confirmResolutionOnce"));
});
