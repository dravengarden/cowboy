import {
  assert,
  assertEquals,
  assertRejects,
  assertThrows,
} from "jsr:@std/assert";
import {
  confirmBindingOnce,
  matchesBinding,
  parseBindingChoices,
  parseBindingPlan,
  parseBindingReceipt,
  telemetryMutationApi,
} from "./telemetryMutation.ts";

const fixture = JSON.parse(
  Deno.readTextFileSync(
    new URL(
      "../../tests/fixtures/telemetry-binding-surface.json",
      import.meta.url,
    ),
  ),
);
const signal = () => new AbortController().signal;
const response = (value: unknown) =>
  new Response(JSON.stringify(value), {
    headers: { "content-type": "application/json" },
  });

Deno.test("ordinary binding projections agree with Rust and preserve exact request identity", () => {
  const plan = parseBindingPlan(fixture.plan);
  const receipt = parseBindingReceipt(fixture.receipt);
  assertEquals(plan, fixture.plan);
  assertEquals(receipt, fixture.receipt);
  assert(matchesBinding(plan, receipt));
  for (
    const field of ["actor", "endpoint", "token", "step", "observation_digest"]
  ) assert(!JSON.stringify(fixture).includes(field));
});

Deno.test("binding plan decoders reject purpose substitution, schema drift, changed state and counter reuse", () => {
  for (
    const change of [
      { actor: "fake" },
      { action: "record_rejected" },
      { action: "revoke" },
      { schema: 2 },
      { plan_id: "wrong-operation-id-123" },
      { request_digest: "bad" },
      { confirmation_available: 1 },
      { operation: { ...fixture.plan.operation, phase: "completed" } },
      { result_head: { ...fixture.plan.result_head, revision: "0" } },
      { result_head: { ...fixture.plan.result_head, policy_epoch: "2" } },
      { result_head: { ...fixture.plan.result_head, selection: null } },
      { restores_operation_id: "foreign-restoration-123" },
    ]
  ) assertThrows(() => parseBindingPlan({ ...fixture.plan, ...change }));
  for (
    const change of [{ extra: true }, { request_digest: "bad" }, { schema: 2 }]
  ) assertThrows(() => parseBindingReceipt({ ...fixture.receipt, ...change }));
  const next = structuredClone(fixture.plan);
  next.operation.expected = {
    revision: "9007199254740993",
    policy_epoch: "9007199254740994",
    selection: null,
  };
  next.operation.change.policy_epoch = "9007199254740995";
  next.result_head.revision = "9007199254740994";
  next.result_head.policy_epoch = "9007199254740995";
  assertEquals(parseBindingPlan(next).result_head, next.result_head);
  next.result_head.revision = "18446744073709551616";
  assertThrows(() => parseBindingPlan(next));
});

Deno.test("choices are closed bounded exact installations and cannot migrate a Service slot", () => {
  const target = {
    machine_id: "machine-test",
    installation: fixture.plan.result_head.selection,
  };
  const choices = {
    schema: 1 as const,
    confirmation_available: false,
    owner_machine_id: "machine-test",
    targets: [target],
    revoke_available: false,
    restore_operation_id: null,
  };
  assertEquals(parseBindingChoices(choices), choices);
  for (
    const change of [
      { targets: Array(65).fill(target) },
      { targets: [target, target] },
      { owner_machine_id: "other-machine" },
      { targets: [{ ...target, token: "fake" }] },
      { owner_machine_id: null, revoke_available: true },
      {
        owner_machine_id: null,
        restore_operation_id: "restored-operation-123",
      },
      {
        targets: [{
          ...target,
          installation: {
            ...target.installation,
            installation_revision: "old",
          },
        }],
      },
    ]
  ) assertThrows(() => parseBindingChoices({ ...choices, ...change }));
});

Deno.test("receipt correlation binds owners and full request, unresolved phases are not silently completed", () => {
  const plan = parseBindingPlan(fixture.plan);
  const receipt = parseBindingReceipt(fixture.receipt);
  for (
    const change of [
      { operation_id: "other-operation-id-123" },
      { machine_id: "other-machine" },
      { expected: fixture.plan.result_head },
      { change: { kind: "revoke", policy_epoch: "1" } },
    ]
  ) {
    assert(
      !matchesBinding(
        plan,
        parseBindingReceipt({
          ...fixture.receipt,
          operation: { ...fixture.receipt.operation, ...change },
        }),
      ),
    );
  }
  assert(
    !matchesBinding(plan, {
      ...receipt,
      request_digest: `sha256:${"a".repeat(64)}`,
    }),
  );
  for (
    const phase of [
      "prepared",
      "dispatching",
      "needs_attention",
      "rejected",
      "completed",
    ]
  ) {
    const result = parseBindingReceipt({
      ...fixture.receipt,
      operation: {
        ...fixture.receipt.operation,
        phase,
        attention: phase === "needs_attention" ? "uncertain" : null,
      },
    });
    assert(matchesBinding(plan, result));
    assertEquals(result.operation.phase, phase);
  }
});

Deno.test("binding confirmation sends one finite reference and ambiguous HTTP permits only exact durable GET", async () => {
  const previous = globalThis.fetch;
  const calls: Array<{ path: string; method: string; body: unknown }> = [];
  const plan = parseBindingPlan(fixture.plan);
  try {
    globalThis.fetch = (input, init) => {
      calls.push({
        path: String(input),
        method: String(init?.method),
        body: init?.body ? JSON.parse(String(init.body)) : null,
      });
      if (init?.method === "POST") {
        return Promise.reject(new Error("lost HTTP"));
      }
      return Promise.resolve(response(fixture.receipt));
    };
    assertEquals(
      await confirmBindingOnce(plan, signal(), signal),
      parseBindingReceipt(fixture.receipt),
    );
    assertEquals(calls, [
      {
        path: "/api/telemetry/binding/confirm",
        method: "POST",
        body: { plan_id: plan.plan_id, action: "select" },
      },
      {
        path: `/api/telemetry/binding/operations/${plan.plan_id}/receipt`,
        method: "GET",
        body: null,
      },
    ]);
  } finally {
    globalThis.fetch = previous;
  }
});

Deno.test("missing receipt or ended observation does not replay binding or claim failure as success", async () => {
  const previous = globalThis.fetch;
  let calls = 0;
  try {
    globalThis.fetch = () => {
      calls++;
      return Promise.resolve(new Response("missing", { status: 404 }));
    };
    const plan = parseBindingPlan(fixture.plan);
    assertEquals(await confirmBindingOnce(plan, signal(), signal), null);
    assertEquals(calls, 2);
    calls = 0;
    assertEquals(
      await confirmBindingOnce(plan, signal(), () => {
        throw new Error("view ended");
      }),
      null,
    );
    assertEquals(calls, 1);
    globalThis.fetch = () =>
      Promise.resolve(
        response({
          ...fixture.receipt,
          request_digest: `sha256:${"a".repeat(64)}`,
        }),
      );
    assertEquals(await confirmBindingOnce(plan, signal(), signal), null);
    globalThis.fetch = () =>
      Promise.resolve(
        new Response("x".repeat(65537), {
          headers: { "content-type": "application/json" },
        }),
      );
    assertEquals(await confirmBindingOnce(plan, signal(), signal), null);
  } finally {
    globalThis.fetch = previous;
  }
});

Deno.test("preview transport independently checks the exact requested target or restoration", async () => {
  const previous = globalThis.fetch;
  try {
    globalThis.fetch = () => Promise.resolve(response(fixture.plan));
    await assertRejects(() =>
      telemetryMutationApi.plan({
        action: "select",
        target: {
          machine_id: "other-machine",
          installation: fixture.plan.result_head.selection,
        },
      }, signal())
    );
    await assertRejects(() =>
      telemetryMutationApi.plan({
        action: "restore",
        operation_id: "old-forward-operation",
      }, signal())
    );
    assertEquals(
      await telemetryMutationApi.plan({
        action: "select",
        target: {
          machine_id: "machine-test",
          installation: fixture.plan.result_head.selection,
        },
      }, signal()),
      fixture.plan,
    );
  } finally {
    globalThis.fetch = previous;
  }
});

Deno.test("core binding surface retains separate confirmation, synchronous one-use ownership and honest fence copy", () => {
  const source = Deno.readTextFileSync(
    new URL("./TelemetryMutationPanel.tsx", import.meta.url),
  ).replace(/\s+/g, " ");
  for (
    const text of [
      "ConfirmSheet",
      "submitted.current = value.plan_id",
      "cowboy:product-sign-out",
      "++sequence.current",
      "pending.current?.abort()",
      "plan?.confirmation_available",
      'phase === "completed"',
      "Already emitted telemetry cannot be undone",
      "closes legacy export admission",
      "background export grant",
      "original one-minute preview",
    ]
  ) assert(source.includes(text), text);
  assert(!source.includes("telemetryRecoveryApi"));
  assert(!source.includes("telemetryBindingApi.resolve"));
});
