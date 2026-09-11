/** Real React + MUI DOM acceptance, hermetic fake effects only. */
import { createElement, StrictMode, useEffect } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import type {
  EffectCapability,
  EffectSchema,
  ProviderHostContext,
} from "@cowboy/provider-ui";
import { ProviderSurface } from "./ProviderSurface";
import { providerUiManifestFixture } from "./providerUiContract.fixture";

function check(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
async function settle(): Promise<void> {
  await new Promise<void>((resolve) => setTimeout(resolve, 20));
}
function pending() {
  let resolve!: () => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<void>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
export async function runProviderUiBrowserConformance(): Promise<string[]> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const tests: string[] = [];
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
  let mounts = 0;
  function Probe() {
    useEffect(() => {
      mounts++;
    }, []);
    return null;
  }
  let manifest = providerUiManifestFixture();
  const render = (
    ownerKey: string | undefined,
    effect: (effect: EffectSchema) => Promise<void>,
    blockedCapabilities?: ReadonlySet<EffectCapability>,
  ) => {
    flushSync(() =>
      root.render(
        createElement(
          StrictMode,
          {},
          createElement(Probe),
          createElement(ProviderSurface, {
            manifest,
            slot: "empty",
            host,
            ownerKey,
            onEffect: effect,
            blockedCapabilities,
          }),
        ),
      )
    );
  };
  function install(): HTMLButtonElement {
    const button = [...container.querySelectorAll("button")].find((node) =>
      node.textContent?.includes("Install")
    );
    check(button, "expected Install button");
    return button;
  }
  try {
    const first = pending();
    let calls = 0;
    const invoke = () => {
      calls++;
      return first.promise;
    };
    render("machine-a/release-a", invoke);
    await settle();
    check(
      mounts >= 2,
      "fixture must exercise React development StrictMode replay",
    );
    check(!install().disabled, "StrictMode owner must be active");
    install().click();
    install().click();
    await settle();
    check(
      calls === 1 && install().disabled,
      "duplicate click escaped synchronous admission",
    );
    tests.push("StrictMode replay + same-stack double click admits one effect");

    manifest = structuredClone(manifest);
    render("machine-a/release-a", invoke);
    await settle();
    check(
      install().disabled,
      "identical Catalog refresh reset an in-flight owner",
    );
    install().click();
    check(calls === 1, "identical refresh allowed duplicate effect");
    tests.push("equal manifest refresh retains in-flight ownership");

    const second = pending();
    let secondCalls = 0;
    const invokeSecond = () => {
      secondCalls++;
      return second.promise;
    };
    manifest = { ...manifest, version: "1.0.1" };
    render("machine-b/release-b", invokeSecond);
    await settle();
    check(
      !install().disabled,
      "replacement binding inherited old pending state",
    );
    install().click();
    first.reject(new Error("retired private diagnostic"));
    await settle();
    check(
      secondCalls === 1 && install().disabled,
      "old rejection cleared replacement pending state",
    );
    check(
      !container.textContent?.includes("retired private diagnostic"),
      "old failure leaked into new surface",
    );
    second.resolve();
    await settle();
    check(!install().disabled, "replacement success did not settle");
    tests.push(
      "new target/release fences late failure without clearing its own busy state",
    );

    render(
      "machine-b/release-b",
      invokeSecond,
      new Set(["install_on_machine"]),
    );
    await settle();
    check(
      ![...container.querySelectorAll("button")].some((node) =>
        node.textContent?.includes("Install")
      ),
      "blocked capability remained actionable",
    );
    check(secondCalls === 1, "permission change dispatched an effect");
    tests.push(
      "committed blocked capability hides even a later effect reducer",
    );

    render(undefined, invokeSecond);
    await settle();
    check(
      install().disabled,
      "unbound presentation can invoke a privileged effect",
    );
    install().click();
    check(secondCalls === 1, "missing target binding dispatched an effect");
    tests.push("read-only presentation has no implicit execution target");

    const last = pending();
    render("machine-c/release-c", () => last.promise);
    await settle();
    install().click();
    flushSync(() => root.render(null));
    let freshCalls = 0;
    render("machine-d/release-d", async () => {
      freshCalls++;
    });
    await settle();
    last.resolve();
    await settle();
    check(!install().disabled, "unmounted completion affected a fresh view");
    install().click();
    await settle();
    check(
      freshCalls === 1 && !install().disabled,
      "fresh owner is unusable after old drain",
    );
    tests.push(
      "unmount drains old effect while a fresh owner remains independently usable",
    );
    return tests;
  } finally {
    flushSync(() => root.unmount());
    container.remove();
  }
}
