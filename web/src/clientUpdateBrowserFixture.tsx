// Real update hook; a controlled download and reload sink avoid navigation.
import { createElement, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import {
  ConnectionBanner,
  type ConnectionStore,
  createConnectionStore,
  useAutoUpdate,
} from "@cowboy/app-shell";

const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
export async function runClientUpdateFixture() {
  const host = document.getElementById("root") ?? document.body.appendChild(document.createElement("div"));
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
  results.push(...await connectionUpdateSignals(host));
  results.push(await bannerKeyboardAction(host));
  return results;
}

/** Desktop names the bar's key in its pill and presses the bar through
 *  `data-update-action`; the press is the bar's own (no second control). */
async function bannerKeyboardAction(host: HTMLElement): Promise<string> {
  let reloads = 0;
  const store = {
    useConnectionBanner: () => ({ kind: "update" as const }),
    downloadUpdate: () => Promise.resolve("ready" as const),
    reloadIntoUpdate: async () => { reloads++; },
  } as unknown as ConnectionStore;
  const root = createRoot(host);
  try {
    root.render(createElement(ConnectionBanner, {
      store,
      automatic: false,
      kinds: ["update"],
      shortcut: "⌘R",
    }));
    await wait(200);
    const bar = document.querySelector<HTMLElement>("[data-update-action]");
    if (bar?.getAttribute("data-update-action") !== "Reload") {
      throw new Error(`ready bar must expose its Reload action: ${bar?.outerHTML.slice(0, 200)}`);
    }
    if (!bar.querySelector("kbd")?.textContent?.includes("⌘R")) {
      throw new Error("the pill must name the keyboard shortcut");
    }
    if (bar.querySelectorAll("button").length !== 0) {
      throw new Error("the key is decoration, not a nested control");
    }
    bar.click();
    await wait(100);
    if (reloads !== 1) throw new Error("pressing the marked bar must reload once");
    return "banner-keyboard-action";
  } finally { root.unmount(); }
}

async function connectionUpdateSignals(host: HTMLElement): Promise<string[]> {
  let available: string | undefined;
  let reloads = 0;
  const downloads: (() => void)[] = [];
  const store = createConnectionStore({
    versionUrl: "/unused-version",
    updateProbe: async () => ({ current: "loaded", available }),
  });
  store.downloadUpdate = () => new Promise((resolve) => downloads.push(() => resolve("ready")));
  store.reloadIntoUpdate = async () => { reloads++; };
  function Probe() {
    const update = useAutoUpdate(store, { countdownSecs: 0 });
    return createElement("div", null, `${update.pending}:${update.phase}`);
  }
  const root = createRoot(host);
  try {
    root.render(createElement(StrictMode, null, createElement(Probe)));
    await wait(100);
    store.connectionReady();
    store.connectionLost();
    store.updateAvailable();
    await store.checkForUpdate("focus");
    await wait(100);
    if (downloads.length || reloads) throw new Error("same-build signals triggered an update");
    available = "next";
    await store.checkForUpdate();
    await wait(100);
    if (downloads.length !== 1) throw new Error("new build was not downloaded");
    available = "newer";
    await store.checkForUpdate();
    await wait(100);
    downloads[0]!();
    await wait(100);
    if (reloads) throw new Error("superseded download triggered a reload");
    if (Number(downloads.length) !== 2) throw new Error("newer build did not replace pending download");
    downloads[1]!();
    await wait(100);
    store.updateAvailable();
    store.connectionReady();
    await wait(100);
    if (reloads !== 1) throw new Error("confirmed build must reload exactly once");
    return ["same-build-signals", "superseded-download"];
  } finally { root.unmount(); }
}
