import {
  assert,
  assertEquals,
  assertRejects,
  assertThrows,
} from "jsr:@std/assert";
import { defineProviderUiContract } from "@cowboy/provider-authoring";
import type { ProviderHostContext, UiNode } from "@cowboy/provider-ui";
import {
  createProviderUiOwner,
  type ProviderUiOwner,
} from "./providerUiOwner.ts";
import {
  providerUiContractFixture,
  providerUiManifestFixture,
} from "./providerUiContract.fixture.ts";

const host: ProviderHostContext = {
  provider_version: "1.0.0",
  installation_state: "available",
  authentication_state: "signed_out",
  distribution_state: "none",
  machine_name: "fixture",
  error_detail: "",
  installed: false,
  auth_ready: false,
  auth_required: false,
  machine_online: true,
  upgrade_available: false,
};
function button(owner: ProviderUiOwner, index = 0) {
  const surface = owner.manifest.ui.surfaces.empty;
  assert(surface.component === "stack");
  const node = surface.children[index];
  assert(node?.component === "button");
  return node;
}
function owner() {
  return createProviderUiOwner(providerUiManifestFixture(), "empty");
}

Deno.test("authoring helper preserves the exact data-only wire object", () => {
  const before = JSON.stringify(providerUiContractFixture);
  assert(
    defineProviderUiContract(providerUiContractFixture) ===
      providerUiContractFixture,
  );
  assertEquals(JSON.stringify(providerUiContractFixture), before);
});

Deno.test("UI owner admits once before same-stack and subscriber reentry", async () => {
  const current = owner();
  const completion = Promise.withResolvers<void>();
  let calls = 0;
  current.updateContext({
    host,
    onEffect: () => {
      calls++;
      void current.emit(button(current));
      return completion.promise;
    },
  });
  let subscribed = false;
  current.subscribe(() => {
    if (current.snapshot().busyEffect) {
      subscribed = true;
      void current.emit(button(current));
      assertEquals(current.lifecycle().tasks, 1);
    }
  });
  const request = current.emit(button(current));
  await current.emit(button(current));
  await current.emit(button(current, 1));
  assert(subscribed);
  assertEquals(calls, 1);
  assertEquals(current.snapshot().state, { busy: true, count: 1, detail: "" });
  assertEquals(current.buttonState(button(current, 1)).disabled, true);
  completion.resolve();
  await request;
  assertEquals(current.snapshot().state.busy, false);
  assertEquals(current.snapshot().busyEffect, null);
  await current.dispose();
});

Deno.test("effect lookup includes pure reducers before the effect rule", async () => {
  const current = owner();
  let calls = 0;
  current.updateContext({
    host,
    blockedCapabilities: new Set(["install_on_machine"]),
    onEffect: async () => {
      calls++;
    },
  });
  assertEquals(current.buttonState(button(current)).effect?.id, "install");
  assertEquals(current.buttonState(button(current)).blocked, true);
  await current.emit(button(current));
  assertEquals(calls, 0);
  assertEquals(current.snapshot().state.count, 0);
  await current.dispose();
});

Deno.test("admission rechecks committed visibility, enabled state and blocked capabilities", async () => {
  const current = owner();
  let calls = 0;
  const invoke = async () => {
    calls++;
  };
  current.updateContext({
    host: { ...host, machine_online: false },
    onEffect: invoke,
  });
  await current.emit(button(current));
  current.updateContext({
    host,
    blockedCapabilities: new Set(["install_on_machine"]),
    onEffect: invoke,
  });
  await current.emit(button(current));
  assertEquals(calls, 0);
  current.updateContext({ host, onEffect: invoke });
  await current.emit(button(current));
  assertEquals(calls, 1);
  await current.dispose();
});

Deno.test("missing host dispatch cannot run an effect or its optimistic reducer", async () => {
  const current = owner();
  current.updateContext({ host });
  await current.emit(button(current));
  assertEquals(current.snapshot().state.count, 0);
  await current.dispose();
});

Deno.test("only the owner's exact surface nodes can emit, never forged/cross-owner buttons", async () => {
  const current = owner();
  const other = owner();
  let calls = 0;
  current.updateContext({
    host,
    onEffect: async () => {
      calls++;
    },
  });
  await current.emit(button(other));
  await current.emit(structuredClone(button(current)));
  assertEquals(calls, 0);
  await Promise.all([current.dispose(), other.dispose()]);
});

Deno.test("input and host mutation cannot alter an admitted immutable UI binding", async () => {
  const input = providerUiManifestFixture();
  const current = createProviderUiOwner(input, "empty");
  const mutableHost = { ...host };
  const blocked = new Set<"install_on_machine">();
  let capability = "";
  current.updateContext({
    host: mutableHost,
    blockedCapabilities: blocked,
    onEffect: async (effect) => {
      capability = effect.capability;
    },
  });
  input.logic.effects[0]!.capability = "logout_service_authentication";
  mutableHost.machine_online = false;
  blocked.add("install_on_machine");
  assertThrows(() => {
    current.snapshot().state.busy = "corrupt";
  }, TypeError);
  assertThrows(() => {
    current.manifest.logic.state[0]!.initial = "corrupt";
  }, TypeError);
  await current.emit(button(current));
  assertEquals(capability, "install_on_machine");
  await current.dispose();
});

Deno.test("dispose fences late success, drains exactly once, and never cancels/replays a host effect", async () => {
  const current = owner();
  const completion = Promise.withResolvers<void>();
  let calls = 0;
  current.updateContext({
    host,
    onEffect: () => {
      calls++;
      return completion.promise;
    },
  });
  const request = current.emit(button(current));
  const snapshot = current.snapshot();
  let observed = 0;
  current.subscribe(() => {
    observed++;
  });
  const disposal = current.dispose();
  assert(disposal === current.dispose());
  assertEquals(current.lifecycle().phase, "draining");
  await current.emit(button(current));
  completion.resolve();
  await request;
  await disposal;
  assert(current.snapshot() === snapshot);
  assertEquals(observed, 0);
  assertEquals(calls, 1);
  assertEquals(current.lifecycle().phase, "disposed");
});

Deno.test("late failure stays on its old task and cannot clear a replacement owner's busy state", async () => {
  const old = owner();
  const newer = owner();
  const first = Promise.withResolvers<void>();
  const second = Promise.withResolvers<void>();
  old.updateContext({ host, onEffect: () => first.promise });
  newer.updateContext({ host, onEffect: () => second.promise });
  const request = old.emit(button(old));
  const rejected = assertRejects(() => request, Error, "private diagnostic");
  const disposal = old.dispose();
  const next = newer.emit(button(newer));
  first.reject(new Error("private diagnostic"));
  await rejected;
  await disposal;
  assertEquals(newer.snapshot().busyEffect, "install");
  assertEquals(newer.snapshot().state.detail, "");
  second.resolve();
  await next;
  await newer.dispose();
});

Deno.test("host failure is settled once and raw diagnostic is not copied into Plugin state", async () => {
  const current = owner();
  current.updateContext({
    host,
    onEffect: () => {
      throw new Error("private diagnostic");
    },
  });
  await assertRejects(
    () => current.emit(button(current)),
    Error,
    "private diagnostic",
  );
  assertEquals(current.snapshot().state.detail, "Provider operation failed");
  assertEquals(current.snapshot().busyEffect, null);
  await current.dispose();
});

Deno.test("core callback observations are fenced without cancelling its submitted operation", async () => {
  const current = owner();
  const completion = Promise.withResolvers<void>();
  let observed = 0;
  let completed = 0;
  current.updateContext({
    host,
    onEffect: async (_effect, observation) => {
      assert(observation.active);
      await completion.promise;
      completed++;
      if (observation.active) observed++;
    },
  });
  const request = current.emit(button(current));
  const disposed = current.dispose();
  completion.resolve();
  await Promise.all([request, disposed]);
  assertEquals(completed, 1);
  assertEquals(observed, 0);
});

Deno.test("observer exceptions do not turn a successful host effect into failure", async () => {
  const current = owner();
  let calls = 0;
  current.updateContext({
    host,
    onEffect: async () => {
      calls++;
    },
  });
  current.subscribe(() => {
    throw new Error("observer");
  });
  await current.emit(button(current));
  assertEquals(calls, 1);
  assertEquals(current.snapshot().state.detail, "");
  await current.dispose();
});

for (const profile of ["request", "success", "failure", "chain"] as const) {
  Deno.test(`unsupported ${profile} profile fails before effect dispatch`, async () => {
    const input = providerUiManifestFixture();
    if (profile === "request") {
      input.logic.effects[0]!.request = { argument: "string" };
    }
    if (profile === "success") {
      input.logic.messages.find((m) => m.id === "done")!.payload = {
        value: "bool",
      };
    }
    if (profile === "failure") {
      // Keep reducer references structurally valid while widening completion.
      input.logic.messages.find((m) => m.id === "failed")!.payload.extra =
        "bool";
    }
    if (profile === "chain") {
      input.logic.reducers.find((r) => r.message === "done")!.effect = "docs";
    }
    const current = createProviderUiOwner(input, "empty");
    let calls = 0;
    current.updateContext({
      host,
      onEffect: async () => {
        calls++;
      },
    });
    assertEquals(current.snapshot().problem, "unsupported_effect_profile");
    await current.emit(button(current));
    assertEquals(calls, 0);
    assertEquals(current.snapshot().state.count, 0);
    await current.dispose();
  });
}

Deno.test("invalid initial state is rejected before ownership or execution", () => {
  const input = providerUiManifestFixture();
  input.logic.state[0]!.initial = "false";
  assertThrows(() => createProviderUiOwner(input, "empty"));
});

Deno.test("pure messages remain local and need no host callback", async () => {
  const input = providerUiManifestFixture();
  input.ui.surfaces.empty = {
    component: "button",
    style: "secondary",
    label: { source: "literal", value: "Reset" },
    emit: { message: "reset", payload: {} },
  };
  const current = createProviderUiOwner(input, "empty");
  current.updateContext({ host });
  const node = current.manifest.ui.surfaces.empty;
  assert(node.component === "button");
  await current.emit(node);
  assertEquals(current.lifecycle().tasks, 0);
  await current.dispose();
});

Deno.test("other surfaces are not an emission authority", async () => {
  const input = providerUiManifestFixture();
  const current = createProviderUiOwner(input, "settings");
  current.updateContext({
    host,
    onEffect: async () => {
      throw new Error("must not execute");
    },
  });
  const surface: UiNode = input.ui.surfaces.empty;
  assert(surface.component === "stack");
  const node = surface.children[0];
  assert(node?.component === "button");
  await current.emit(node);
  await current.dispose();
});
