import {
  assert,
  assertEquals,
  assertRejects,
  assertStringIncludes,
  assertThrows,
} from "jsr:@std/assert";
import { createProviderAuthenticationOwner } from "./providerAuthenticationOwner.ts";
import { createProviderDialogOwner } from "./providerDialogOwner.ts";
import { createProviderUninstallOwner } from "./providerUninstallOwner.ts";
import {
  deferredFixture,
  managementEntryFixture,
  uninstallPlanFixture,
} from "./providerManagement.fixture.ts";

async function settle() {
  for (let i = 0; i < 32; i++) await Promise.resolve();
}
function transport() {
  const calls: {
    url: string;
    init: RequestInit;
    reply: ReturnType<typeof deferredFixture<Response>>;
  }[] = [];
  return {
    calls,
    fetch(url: string, init: RequestInit = {}): Promise<Response> {
      const reply = deferredFixture<Response>();
      calls.push({ url, init, reply });
      return reply.promise; // Deliberately ignores abort, to test late settlement.
    },
    reply(index: number, value: unknown, status = 200) {
      calls[index]!.reply.resolve(Response.json(value, { status }));
    },
    drain() {
      for (const call of calls) call.reply.reject(new Error("fixture closed"));
    },
  };
}
function clock() {
  const timers = new Set<{ delay: number; callback: () => void }>();
  return {
    timers,
    schedule(callback: () => void, delay: number) {
      const timer = { delay, callback };
      timers.add(timer);
      return () => {
        timers.delete(timer);
      };
    },
    fire(delay: number) {
      const timer = [...timers].find((timer) => timer.delay === delay);
      assert(timer, `expected ${delay}ms timer`);
      timers.delete(timer);
      timer.callback();
    },
  };
}
function authFixture() {
  const http = transport(), time = clock();
  let closes = 0, refreshes = 0;
  const copy = deferredFixture<boolean>();
  const owner = createProviderAuthenticationOwner({
    fetch: http.fetch,
    executor: (id) => managementEntryFixture(id),
    refresh: () => {
      refreshes++;
      return Promise.resolve();
    },
    closeBrowser: () => {
      closes++;
    },
    copy: () => copy.promise,
  }, time.schedule);
  const open = (id = "example") =>
    owner.open({
      provider: managementEntryFixture(id),
      sharedProviderNames: [id],
      credentialTitle: id,
    });
  return {
    ...http,
    time,
    owner,
    open,
    copy,
    closes: () => closes,
    refreshes: () => refreshes,
    async start(id = "example", request = "request-a") {
      open(id);
      const index = http.calls.length;
      const task = owner.start("key");
      http.reply(index, {
        request_id: request,
        expires_at_ms: 1_999_999_999_999,
      });
      await task;
      return http.calls.length - 1; // first read
    },
    async cleanup() {
      const close = owner.dispose();
      http.drain();
      copy.resolve(false);
      await close;
      assertEquals(owner.lifecycle(), {
        phase: "disposed",
        resources: 0,
        tasks: 0,
        failures: 0,
      });
      assertEquals(time.timers.size, 0);
    },
  };
}
function challenge(provider = "example", request = "request-a") {
  return {
    event: "login_challenge",
    provider,
    request_id: request,
    verification_url: "https://example.invalid/login",
    user_code: "FIXTURE-CODE",
    input_required: true,
    secret_input: true,
    expires_at_ms: 1_999_999_999_999,
  };
}
function events(events: unknown[], request_id = "request-a") {
  return { request_id, events };
}

Deno.test("dialog owner reserves admission before observers and drains retired writes", async () => {
  const owner = createProviderDialogOwner<{ id: string }, "write">();
  const first = owner.open({ id: "old" })!;
  const result = deferredFixture<void>();
  let calls = 0;
  owner.subscribe(() => {
    if (owner.snapshot().busy) {
      void first.run("write", "failed", async () => {
        calls++;
      });
    }
  });
  const task = first.run("write", "failed", () => {
    calls++;
    return result.promise;
  });
  assertEquals(calls, 1);
  const second = owner.open({ id: "new" })!;
  assert(!first.active);
  assert(second.active);
  const close = owner.dispose();
  assert(close === owner.dispose());
  assertEquals(owner.lifecycle().phase, "draining");
  assertEquals(owner.lifecycle().resources, 2);
  result.resolve();
  await task;
  await close;
  assertEquals(owner.lifecycle().resources, 0);
});

Deno.test("dialog observers cannot mutate consent or prevent an admitted request", async () => {
  const owner = createProviderDialogOwner<
    { nested: { consent: boolean } },
    "write"
  >();
  const lease = owner.open({ nested: { consent: false } })!;
  owner.subscribe(() => {
    throw new Error("bad renderer");
  });
  assertThrows(() => {
    owner.snapshot().value!.nested.consent = true;
  }, TypeError);
  let called = false;
  await lease.run("write", "failed", () => {
    called = true;
    return Promise.resolve();
  });
  assert(called);
  await owner.dispose();
});

Deno.test("dialog resource budget and failing cleanup retain honest ownership", async () => {
  const owner = createProviderDialogOwner<number, "write">();
  const deferred = deferredFixture<void>();
  for (let i = 0; i < 16; i++) {
    const lease = owner.open(i)!;
    void lease.run("write", "failed", () => deferred.promise);
  }
  assertEquals(owner.open(17), undefined);
  assertStringIncludes(owner.snapshot().error, "draining");
  owner.current()!.defer(() => {
    throw new Error("cleanup failure");
  });
  const close = owner.dispose();
  deferred.resolve();
  await assertRejects(() => close, AggregateError);
  assertEquals(owner.lifecycle().phase, "needs_reconcile");
  assertEquals(owner.lifecycle().resources, 1);
});

Deno.test("sign-in start is synchronous single-flight and sends the selected immutable release", async () => {
  const f = authFixture();
  try {
    f.open();
    const task = f.owner.start("key");
    await f.owner.start("key");
    assertEquals(f.calls.length, 1);
    assertEquals(f.owner.snapshot().busy, "start");
    assertEquals(f.owner.snapshot().value?.pendingMethod, "key");
    assertEquals(JSON.parse(f.calls[0]!.init.body as string), {
      method: "key",
      provider_version: "1.0.0",
      generation_digest: `sha256:${"2".repeat(64)}`,
    });
    f.reply(0, { request_id: "request-a", expires_at_ms: 1_999_999_999_999 });
    await task;
    assertEquals(f.calls[0]!.init.signal, undefined);
    assertEquals(f.calls[1]!.init.signal?.aborted, false);
  } finally {
    await f.cleanup();
  }
});

for (const success of [true, false]) {
  Deno.test(`old sign-in start ${success ? "success" : "failure"} cannot bind or clear a new flow`, async () => {
    const f = authFixture();
    try {
      f.open();
      const first = f.owner.start("key");
      f.owner.dismiss();
      f.open("other");
      const second = f.owner.start("key");
      f.reply(
        0,
        success
          ? { request_id: "old", expires_at_ms: 1_999_999_999_999 }
          : { error: "old private detail" },
        success ? 200 : 409,
      );
      await first;
      assertEquals(
        f.owner.snapshot().value?.flow.provider.provider_id,
        "other",
      );
      assertEquals(f.owner.snapshot().value?.flow.requestId, undefined);
      assertEquals(f.owner.snapshot().busy, "start");
      assertEquals(f.owner.snapshot().error, "");
      assertEquals(f.calls.length, 2); // no old polling or automatic cancellation
      assertEquals(f.refreshes(), 0);
      f.reply(1, { request_id: "new", expires_at_ms: 1_999_999_999_999 });
      await second;
    } finally {
      await f.cleanup();
    }
  });
}

Deno.test("sign-in observation uses one bounded read, including after abort is ignored", async () => {
  const f = authFixture();
  try {
    await f.start();
    assertEquals([...f.time.timers].map((timer) => timer.delay), [8000]);
    f.time.fire(8000);
    await settle();
    assert(f.calls[1]!.init.signal?.aborted);
    assertEquals(f.calls.length, 2);
    assertEquals(f.time.timers.size, 0);
    f.calls[1]!.reply.reject(new Error("deadline"));
    await settle();
    assertEquals([...f.time.timers].map((timer) => timer.delay), [750]);
    f.time.fire(750);
    assertEquals(f.calls.length, 3);
    f.owner.dismiss();
    assert(f.calls[2]!.init.signal?.aborted);
    assertEquals(f.time.timers.size, 0);
  } finally {
    await f.cleanup();
  }
});

Deno.test("late expired-response body cannot close a replacement sign-in browser", async () => {
  const f = authFixture();
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  try {
    await f.start();
    f.calls[1]!.reply.resolve(
      new Response(
        new ReadableStream<Uint8Array>({
          start(value) {
            controller = value;
          },
        }),
        { status: 404 },
      ),
    );
    await settle();
    f.open("other");
    controller.enqueue(new TextEncoder().encode("old ended"));
    controller.close();
    await settle();
    assertEquals(f.closes(), 0);
    assertEquals(f.owner.snapshot().error, "");
    assertEquals(f.owner.snapshot().value?.flow.provider.provider_id, "other");
    assertEquals(f.time.timers.size, 0);
  } finally {
    await f.cleanup();
  }
});

Deno.test("unmount drains a submitted start without DELETE, native close, or late polling", async () => {
  const f = authFixture();
  f.open();
  const task = f.owner.start("key");
  const close = f.owner.dispose();
  assertEquals(f.owner.lifecycle().phase, "draining");
  f.reply(0, { request_id: "late", expires_at_ms: 1_999_999_999_999 });
  await task;
  await close;
  assertEquals(f.calls.length, 1);
  assertEquals(f.closes(), 0);
  assertEquals(f.owner.snapshot().value, null);
  await f.cleanup();
});

Deno.test("code submission is single-flight, surfaces rejection, and retains editable input", async () => {
  const f = authFixture();
  try {
    const poll = await f.start();
    f.reply(poll, events([challenge()]));
    await settle();
    f.owner.setInput(" fixture-secret ");
    const task = f.owner.submit("Submit failed");
    await f.owner.submit("Submit failed");
    f.owner.setInput("racing edit");
    assertEquals(f.calls.length, 3);
    assertEquals(f.owner.snapshot().busy, "submit");
    assertEquals(JSON.parse(f.calls[2]!.init.body as string), {
      code: "fixture-secret",
    });
    f.reply(2, { detail: "Fixture input rejected" }, 409);
    await task;
    assertEquals(f.owner.snapshot().value?.input, " fixture-secret ");
    assertEquals(f.owner.snapshot().error, "Fixture input rejected");
    assertEquals(f.owner.snapshot().busy, null);
  } finally {
    await f.cleanup();
  }
});

Deno.test("newest durable sign-in success outranks an older submit failure and clipboard result", async () => {
  const f = authFixture();
  try {
    const poll = await f.start();
    f.reply(poll, events([challenge()]));
    await settle();
    f.owner.setInput("fixture");
    const submit = f.owner.submit("Submit failed");
    f.owner.copyCode();
    f.time.fire(750);
    f.reply(
      3,
      events([challenge(), {
        event: "login_state",
        provider: "example",
        request_id: "request-a",
        state: "signed_in",
      }]),
    );
    await settle();
    f.reply(2, { detail: "stale submit failure" }, 409);
    await submit;
    f.copy.resolve(true);
    await settle();
    assertEquals(f.owner.snapshot().value?.input, "");
    assertEquals(f.owner.snapshot().value?.clipboardNotice, "");
    assertEquals(f.owner.snapshot().error, "");
    assertEquals(f.closes(), 1);
    assertEquals(f.time.timers.size, 0);
    await f.owner.cancel();
    assertEquals(f.calls.length, 4); // no DELETE after success
  } finally {
    await f.cleanup();
  }
});

for (const back of [true, false]) {
  Deno.test(`explicit ${back ? "back" : "cancel"} rejects once without erasing a failed request`, async () => {
    const f = authFixture();
    try {
      await f.start();
      const task = back ? f.owner.back() : f.owner.cancel();
      await f.owner.cancel();
      await f.owner.back();
      assertEquals(f.calls.length, 3);
      assertEquals(f.calls[2]!.init.method, "DELETE");
      f.reply(2, { detail: "Fixture cancellation was not acknowledged" }, 503);
      await task;
      assertEquals(f.owner.snapshot().value?.flow.requestId, "request-a");
      assertStringIncludes(f.owner.snapshot().error, "not acknowledged");
      assertEquals(f.closes(), 0);
      assertEquals(f.calls.length, 3); // no second read until the old abort settles
      f.calls[1]!.reply.reject(new Error("aborted read settled"));
      await settle();
      assertEquals(f.calls.length, 4); // only a fresh read, not another DELETE
    } finally {
      await f.cleanup();
    }
  });
}

Deno.test("back success returns to methods; dismissing or replacing while cancel waits never changes the next flow", async () => {
  const f = authFixture();
  try {
    await f.start();
    const back = f.owner.back();
    f.calls[2]!.reply.resolve(new Response(null, { status: 202 }));
    await back;
    assertEquals(f.owner.snapshot().value?.flow.requestId, undefined);
    assertEquals(f.owner.snapshot().value?.input, "");
    await f.start("example", "request-b");
    const cancel = f.owner.cancel();
    const index = f.calls.length - 1;
    f.open("other");
    const closes = f.closes();
    f.calls[index]!.reply.resolve(new Response(null, { status: 202 }));
    await cancel;
    assertEquals(f.owner.snapshot().value?.flow.provider.provider_id, "other");
    assertEquals(f.closes(), closes);
  } finally {
    await f.cleanup();
  }
});

for (
  const body of [events([challenge("wrong")]), events([challenge()], "wrong"), {
    events: [],
  }, events([{ ...challenge(), input_required: "yes" }])]
) {
  Deno.test("sign-in reads reject mismatched or ill-typed evidence without following it", async () => {
    const f = authFixture();
    try {
      const poll = await f.start();
      f.reply(poll, body);
      await settle();
      assertStringIncludes(f.owner.snapshot().error, "Invalid Provider");
      assertEquals(f.owner.snapshot().value?.flow.events, []);
      assertEquals(f.time.timers.size, 0);
      assertEquals(f.closes(), 0);
    } finally {
      await f.cleanup();
    }
  });
}

Deno.test("Provider sign-in refuses unknown methods and preserves the replacement clipboard notice", async () => {
  const f = authFixture();
  try {
    f.open();
    await f.owner.start("undeclared");
    assertEquals(f.calls.length, 0);
    assertStringIncludes(f.owner.snapshot().error, "Unknown Provider");
    const poll = await f.start();
    f.reply(poll, events([challenge()]));
    await settle();
    f.owner.copyCode();
    f.open("other");
    f.copy.resolve(false);
    await settle();
    assertEquals(f.owner.snapshot().value?.clipboardNotice, "");
  } finally {
    await f.cleanup();
  }
});

function uninstallFixture() {
  const http = transport();
  const owner = createProviderUninstallOwner(http.fetch, () => 1000);
  return {
    ...http,
    owner,
    async prepare(plan = uninstallPlanFixture()) {
      const index = http.calls.length;
      const task = owner.prepare(plan.machine_id, plan.plugin_id);
      http.reply(index, plan);
      await task;
    },
    async cleanup() {
      const close = owner.dispose();
      http.drain();
      await close;
      assertEquals(owner.lifecycle().phase, "disposed");
      assertEquals(owner.lifecycle().resources, 0);
    },
  };
}

Deno.test("poll expiry outranks a late submit failure and clears only the current request", async () => {
  const f = authFixture();
  try {
    const poll = await f.start();
    f.reply(poll, events([challenge()]));
    await settle();
    f.owner.setInput("fixture");
    const submit = f.owner.submit("Submit failed");
    f.time.fire(750);
    f.calls[3]!.reply.resolve(new Response("Request expired", { status: 410 }));
    await settle();
    f.reply(2, { detail: "stale submit refusal" }, 409);
    await submit;
    assertEquals(f.owner.snapshot().value?.flow.requestId, undefined);
    assertEquals(f.owner.snapshot().error, "Request expired");
    assertEquals(f.owner.snapshot().value?.input, "");
    assertEquals(f.time.timers.size, 0);
  } finally {
    await f.cleanup();
  }
});

Deno.test("failed cancel retries never multiply an abort-ignoring status read", async () => {
  const f = authFixture();
  try {
    await f.start();
    for (let i = 0; i < 5; i++) {
      const task = f.owner.cancel();
      f.reply(f.calls.length - 1, { error: "Not acknowledged" }, 503);
      await task;
      assertEquals(f.calls.filter((call) => !call.init.method).length, 1);
    }
    assertEquals(f.owner.lifecycle().resources, 1);
    f.calls[1]!.reply.reject(new Error("old read finally settled"));
    await settle();
    assertEquals(f.calls.filter((call) => !call.init.method).length, 2);
  } finally {
    await f.cleanup();
  }
});

Deno.test("unknown login states and revoked status access stop observation without cancelling", async () => {
  for (const status of [200, 401, 403]) {
    const f = authFixture();
    try {
      const poll = await f.start();
      f.reply(
        poll,
        events([{
          event: "login_state",
          provider: "example",
          request_id: "request-a",
          state: "future_success",
        }]),
        status,
      );
      await settle();
      assert(f.owner.snapshot().error.length > 0);
      assertEquals(f.owner.snapshot().value?.flow.events, []);
      assertEquals(f.time.timers.size, 0);
      assertEquals(f.calls.length, 2);
      assertEquals(f.closes(), 0);
    } finally {
      await f.cleanup();
    }
  }
});

Deno.test("uninstall preview projection discards unowned fields and freezes nested consent", async () => {
  const f = uninstallFixture();
  try {
    const task = f.owner.prepare("machine-a", "example");
    f.reply(0, {
      ...uninstallPlanFixture(),
      unrelated: "not UI contract data",
    });
    await task;
    const value = f.owner.snapshot().value;
    assert(value?.phase === "ready");
    assert(!("unrelated" in value.plan));
    assertThrows(() => {
      value.plan.active_session_ids.push("different");
    }, TypeError);
    assertThrows(() => {
      value.plan.machine_id = "changed";
    }, TypeError);
  } finally {
    await f.cleanup();
  }
});

Deno.test("uninstall confirmation requires a current unexpired plan and explicit active-session consent", async () => {
  const f = uninstallFixture();
  try {
    await f.owner.confirm();
    assertEquals(f.calls.length, 0);
    await f.prepare();
    await f.owner.confirm();
    assertEquals(f.calls.length, 1);
    assertStringIncludes(f.owner.snapshot().error, "Confirm stopping");
    f.owner.setConfirmActive(true);
    const task = f.owner.confirm();
    await f.owner.confirm();
    assertEquals(f.calls.length, 2);
    assertEquals(f.calls[1]!.init.signal, undefined);
    assertEquals(JSON.parse(f.calls[1]!.init.body as string), {
      plan_id: "plan-a",
      confirm_active_sessions: true,
    });
    f.calls[1]!.reply.resolve(new Response(null, { status: 204 }));
    await task;
    assertEquals(f.owner.snapshot().value, null);
    await f.prepare({ ...uninstallPlanFixture(), expires_at_ms: 999 });
    f.owner.setConfirmActive(true);
    await f.owner.confirm();
    assertEquals(f.calls.length, 3);
    assertStringIncludes(f.owner.snapshot().error, "expired");
  } finally {
    await f.cleanup();
  }
});

for (const status of [200, 409]) {
  Deno.test(`late uninstall ${status} cannot close or poison the next confirmation`, async () => {
    const f = uninstallFixture();
    try {
      await f.prepare();
      f.owner.setConfirmActive(true);
      const old = f.owner.confirm();
      f.owner.close();
      await f.prepare(uninstallPlanFixture("plan-b", "machine-b"));
      f.owner.setConfirmActive(true);
      const next = f.owner.confirm();
      f.reply(1, { error: "old error" }, status);
      await old;
      assertEquals(f.owner.snapshot().busy, "confirm");
      assertEquals(f.owner.snapshot().error, "");
      const value = f.owner.snapshot().value;
      assert(value?.phase === "ready");
      assertEquals(value.plan.plan_id, "plan-b");
      f.reply(3, { error: "new rejection" }, 409);
      await next;
      assertEquals(f.owner.snapshot().error, "new rejection");
    } finally {
      await f.cleanup();
    }
  });
}

Deno.test("plan preparation is latest-wins and retired Provider surfaces cannot open a confirmation", async () => {
  const f = uninstallFixture();
  try {
    const old = f.owner.prepare("machine-a", "example");
    await f.prepare(uninstallPlanFixture("plan-b", "machine-b"));
    f.reply(0, uninstallPlanFixture());
    await old;
    const value = f.owner.snapshot().value;
    assert(value?.phase === "ready");
    assertEquals(value.plan.plan_id, "plan-b");
    const observation = { active: true };
    const task = f.owner.prepare("machine-a", "example", observation);
    observation.active = false;
    f.reply(2, uninstallPlanFixture());
    await task;
    assertEquals(f.owner.snapshot().value, null);
  } finally {
    await f.cleanup();
  }
});

for (
  const patch of [{ machine_id: "wrong" }, { plugin_id: "wrong" }, {
    active_session_ids: ["unknown"],
  }, { expires_at_ms: null }]
) {
  Deno.test("malformed or retargeted uninstall previews cannot acquire confirmation", async () => {
    const f = uninstallFixture();
    try {
      const task = f.owner.prepare("machine-a", "example");
      f.reply(0, { ...uninstallPlanFixture(), ...patch });
      await assertRejects(() => task, Error, "Invalid Provider uninstall plan");
      await f.owner.confirm();
      assertEquals(f.calls.length, 1);
    } finally {
      await f.cleanup();
    }
  });
}
