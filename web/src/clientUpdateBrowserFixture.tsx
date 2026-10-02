// Real update hook; a controlled download and reload sink avoid navigation.
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import { type ConnectionStore, useAutoUpdate } from "@cowboy/app-shell";

const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
export async function runClientUpdateFixture() {
  const host = document.getElementById("root")!;
  const results: string[] = [];
  for (const scenario of ["manual", "zero", "three", "change-delay", "rejected-zero", "switch-manual"] as const) {
    let finish!: () => void;
    const download = new Promise<"ready" | "rejected">((resolve) => { finish = () => resolve(scenario === "rejected-zero" ? "rejected" : "ready"); });
    let reloads = 0;
    let pressed: (() => void) | undefined;
    let seconds = scenario === "zero" || scenario === "rejected-zero" ? 0 : 3;
    let automatic = scenario !== "manual";
    const canApplyUpdate = () => scenario !== "zero";
    const store = {
      useConnectionBanner: () => ({ kind: "update" as const }),
      downloadUpdate: () => download,
      reloadIntoUpdate: async () => { reloads++; },
    } as unknown as ConnectionStore;
    function Probe() {
      const update = useAutoUpdate(store, {
        automatic, countdownSecs: seconds,
        canApplyUpdate,
      });
      pressed = update.requestUpdate;
      return createElement("div", null, `${update.phase}:${update.secs}:${update.held}`);
    }
    const root = createRoot(host);
    try {
      root.render(createElement(Probe));
      await wait(100);
      if (reloads) throw new Error("reload before download");
      finish();
      await wait(100);
      if (scenario === "rejected-zero") {
        if (reloads || !host.textContent?.startsWith("rejected")) throw new Error("zero must not apply a rejected build");
      } else if (scenario === "zero") {
        if (reloads !== 1) throw new Error("zero must apply immediately, including while busy");
      } else if (scenario === "manual" || scenario === "switch-manual") {
        if (scenario === "switch-manual") {
          automatic = false;
          root.render(createElement(Probe));
        }
        await wait(3200);
        if (reloads) throw new Error("manual mode auto-reloaded");
        pressed!();
        await wait(100);
        if (reloads !== 1) throw new Error("manual press failed");
      } else if (scenario === "change-delay") {
        seconds = 1;
        root.render(createElement(Probe));
        await wait(1200);
        if (reloads !== 1) throw new Error("changed countdown was not applied live");
      } else {
        await wait(2100);
        if (reloads) throw new Error("three seconds ended early");
        await wait(1100);
        if (reloads !== 1) throw new Error("three-second countdown did not reload once");
      }
      results.push(scenario);
    } finally { root.unmount(); }
  }
  return results;
}
