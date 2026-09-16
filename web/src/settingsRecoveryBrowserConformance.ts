/** Real React/MUI and native IndexedDB, synthetic data in a disposable profile.
 * Local downloads are observed without saving files; all HTTP is a fake GET.
 */
// oxlint-disable promise/avoid-new
import { createElement, type ReactNode, StrictMode } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { createTheme, ThemeProvider } from "@mui/material";
import { ProductSyncDataNotice } from "./ProductSyncDataNotice.tsx";
import { TelemetryBindingPanel } from "./TelemetryBindingPanel.tsx";
import {
  createProductSyncDatabase,
  productSyncDatabase,
} from "./productSyncDatabase.ts";
import { PRODUCT_SESSION_END_EVENT } from "./productSessionEnd.ts";
import { deferredFixture } from "./providerManagement.fixture.ts";
import { SurfaceProvider } from "./surface/SurfaceProfile.tsx";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function settle() {
  await new Promise<void>((resolve) => setTimeout(resolve, 25));
}
async function until(test: () => boolean, message: string) {
  for (let attempt = 0; attempt < 80; attempt++) {
    if (test()) return;
    await settle();
  }
  check(test(), message);
}
function native<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.addEventListener("success", () => resolve(request.result), {
      once: true,
    });
    request.addEventListener("error", () => reject(request.error), {
      once: true,
    });
  });
}

export async function runSettingsRecoveryBrowserConformance(): Promise<
  string[]
> {
  const dbName = `settings-recovery-${crypto.randomUUID()}`;
  const keys = Array.from(
    { length: 376 },
    (_, index) => `cowboy:sync:queue:fixture-${String(index).padStart(3, "0")}`,
  );
  const open = indexedDB.open(dbName, 1);
  open.addEventListener(
    "upgradeneeded",
    () => open.result.createObjectStore("clients"),
  );
  const db = await native(open);
  const seed = db.transaction("clients", "readwrite");
  keys.forEach((key, index) =>
    seed.objectStore("clients").put(
      index === 1 ? new Date(0) : {
        base: { version: 0, value: "private-fixture-not-for-display" },
        pending: [{ id: `pending-${index}`, args: "synthetic prompt" }],
      },
      key,
    )
  );
  await new Promise<void>((resolve, reject) => {
    seed.oncomplete = () => resolve();
    seed.onabort = () => reject(new Error("fixture seed failed"));
  });
  db.close();
  const owner = createProductSyncDatabase(
    () => "fixture-user",
    async () => ({
      schema: "dravengarden.cowboy.product-sync-dataset/v1",
      dataset_id: `dataset-${"1".repeat(64)}`,
      user_id: "fixture-user",
      database_version: 2,
      outbox_contract: "atomic-delta-v1",
    }),
    { dbName },
  );
  const readAll = async () => {
    const db = await native(indexedDB.open(dbName, 2));
    try {
      return JSON.stringify(
        await native(
          db.transaction("clients").objectStore("clients").getAll(),
        ),
      );
    } finally {
      db.close();
    }
  };
  await owner.ready();
  const baseline = await readAll();
  const original = {
    legacyRecords: productSyncDatabase.legacyRecords,
    exportLegacy: productSyncDatabase.exportLegacy,
    discardLegacy: productSyncDatabase.discardLegacy,
    fetch: globalThis.fetch,
    createURL: URL.createObjectURL,
    revokeURL: URL.revokeObjectURL,
    click: HTMLAnchorElement.prototype.click,
  };
  const exports: string[] = [];
  const urls: { url: string; blob: Blob }[] = [];
  const revoked: string[] = [];
  const downloads: { href: string; name: string }[] = [];
  const calls: {
    init: RequestInit;
    reply: ReturnType<typeof deferredFixture<Response>>;
  }[] = [];
  productSyncDatabase.legacyRecords = () => owner.legacyRecords();
  productSyncDatabase.exportLegacy = (key) => {
    exports.push(key);
    return owner.exportLegacy(key);
  };
  URL.createObjectURL = (blob) => {
    check(blob instanceof Blob, "export did not create a Blob");
    const url = original.createURL(blob);
    urls.push({ url, blob });
    return url;
  };
  URL.revokeObjectURL = (url) => {
    revoked.push(url);
    original.revokeURL(url);
  };
  HTMLAnchorElement.prototype.click = function () {
    downloads.push({ href: this.href, name: this.download });
  };
  globalThis.fetch = ((target, init = {}) => {
    check(
      target === "/api/telemetry/binding" &&
        (init.method ?? "GET") === "GET" && init.body === undefined &&
        init.cache === "no-store",
      "settings attempted a network effect or private endpoint read",
    );
    const reply = deferredFixture<Response>();
    calls.push({ init, reply });
    return reply.promise;
  }) as typeof fetch;
  const container = document.createElement("div");
  container.style.width = "360px";
  document.body.append(container);
  const root = createRoot(container);
  // Mobile-width geometry with the product's 44px touch target floor. This is
  // native Firefox, not a claim about physical WebKit/native download behavior.
  const theme = createTheme({
    typography: { fontSize: 16 },
    components: {
      MuiButton: { styleOverrides: { root: { minHeight: 44 } } },
    },
  });
  const render = (node: ReactNode) =>
    flushSync(() =>
      root.render(createElement(
        StrictMode,
        null,
        createElement(
          ThemeProvider,
          { theme },
          createElement(SurfaceProvider, { children: node }),
        ),
      ))
    );
  const button = (label: string) => {
    const value = [...container.querySelectorAll("button")].find((button) =>
      button.textContent === label
    );
    check(value, `missing button: ${label}`);
    return value;
  };
  const click = (label: string) => flushSync(() => button(label).click());
  const select = (index: number) => {
    const select = container.querySelector("select");
    check(select, "missing native record selector");
    flushSync(() => {
      select.value = String(index);
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
  };
  const countExports = () => exports.length;
  const countDownloads = () => downloads.length;
  const countCalls = () => calls.length;
  const absent = () =>
    Response.json({
      schema: 1,
      resolution_admission: "closed",
      journal: { state: "absent" },
    });
  const answerAll = () => {
    for (const call of calls) call.reply.resolve(absent());
  };
  const tests: string[] = [];
  try {
    render(createElement(ProductSyncDataNotice));
    await until(
      () => !!container.textContent?.includes("376 older records"),
      "inventory missing",
    );
    check(
      container.getBoundingClientRect().height < 230,
      "collapsed inventory grew with record count",
    );
    check(
      !container.querySelector("[role=alert]"),
      "retained data became an error banner",
    );
    check(
      !container.querySelector("select"),
      "collapsed inventory mounted its picker",
    );
    click("Review older records");
    check(
      button("Hide older records").getAttribute("aria-expanded") === "true",
      "disclosure state missing",
    );
    check(
      container.querySelectorAll("option").length === 376,
      "selector lost records",
    );
    check(
      container.querySelectorAll("button").length === 5,
      "one button was rendered per record",
    );
    const height = container.getBoundingClientRect().height;
    check(
      height < 560 && container.scrollWidth <= 360,
      "mobile recovery overflows",
    );
    check(
      !container.textContent?.includes("private-fixture") &&
        !container.innerHTML.includes("cowboy:sync:"),
      "private keys or payloads painted",
    );
    check(
      countExports() === 0 && countCalls() === 0,
      "inspection exported or sent data",
    );
    tests.push(
      `376 native IDB records / mobile width 360 / expanded height ${
        Math.ceil(height)
      } / one picker and download / private values never painted`,
    );

    check(button("Previous").disabled, "first record can underflow");
    select(375);
    check(button("Next").disabled, "last record can overflow");
    click("Previous");
    check(
      container.querySelector("select")?.value === "374",
      "previous record failed",
    );
    click("Next");
    const download = button("Download selected record");
    flushSync(() => {
      download.click();
      download.click();
    });
    check(
      countExports() === 1 && exports[0] === keys[375],
      "double click exported twice or the wrong record",
    );
    await until(() => countDownloads() === 1, "download missing");
    check(
      downloads[0]?.name === "cowboy-retained-record-376.json",
      "download filename is ambiguous or private",
    );
    check(urls[0], "missing export URL");
    const body = JSON.parse(await urls[0].blob.text());
    check(
      body.key === keys[375] && body.replay_authorized === false,
      "export changed identity or replay fence",
    );
    check(
      container.textContent?.includes("Download requested for record 376"),
      "download feedback missing",
    );
    tests.push(
      "native selection and previous/next bounds / same-turn double click exports once / numbered private JSON with replay disabled",
    );

    select(1);
    click("Download selected record");
    await until(
      () => !!container.textContent?.includes("could not be downloaded safely"),
      "unsafe value was not reported",
    );
    check(
      container.textContent?.includes("376 older records") &&
        countDownloads() === 1,
      "export failure destroyed inventory or downloaded unsafe data",
    );
    select(2);
    check(
      !container.textContent?.includes("could not be downloaded safely"),
      "old record error survived selection",
    );
    check(
      await readAll() === baseline &&
        (await owner.queueSessions()).length === 0,
      "review mutated, imported or deleted original data",
    );
    tests.push(
      "unsafe record retained without download / failure is per-record / selection clears error / all 376 originals unchanged, no import",
    );

    const signedOut = deferredFixture<string>();
    productSyncDatabase.exportLegacy = () => signedOut.promise;
    click("Download selected record");
    flushSync(() =>
      globalThis.dispatchEvent(new Event(PRODUCT_SESSION_END_EVENT))
    );
    signedOut.resolve("private late response");
    await settle();
    check(
      !container.textContent && countDownloads() === 1 && urls.length === 1,
      "logout allowed a late download or kept old inventory",
    );
    check(
      revoked.filter((url) => url === urls[0]?.url).length === 1,
      "URL leaked or revoked twice",
    );
    tests.push(
      "product-session end clears local recovery and seals pending export / owned URL revoked exactly once",
    );

    render(null);
    render(createElement(ProductSyncDataNotice));
    await until(
      () => !!container.textContent?.includes("376 older records"),
      "fresh inventory missing",
    );
    check(
      !container.querySelector("select"),
      "remount reopened private recovery",
    );
    click("Review older records");
    const unmounted = deferredFixture<string>();
    productSyncDatabase.exportLegacy = () => unmounted.promise;
    click("Download selected record");
    render(null);
    unmounted.resolve("private late response");
    await settle();
    check(
      !container.textContent && countDownloads() === 1 && urls.length === 1,
      "unmount allowed late export",
    );
    tests.push(
      "fresh StrictMode mount starts collapsed / unmount drains late export without another URL or download",
    );

    const discarded: string[] = [];
    // Read the tally through a call so an earlier assertion cannot narrow it.
    const countDiscards = () => discarded.length;
    productSyncDatabase.discardLegacy = (key) => {
      discarded.push(key);
      return owner.discardLegacy(key);
    };
    render(createElement(ProductSyncDataNotice));
    await until(
      () => !!container.textContent?.includes("376 older records"),
      "inventory missing before deletion",
    );
    click("Review older records");
    click("Delete selected record");
    check(
      !!container.textContent?.includes("cannot be recovered") &&
        countDiscards() === 0,
      "arming deleted a record or asked for nothing",
    );
    click("Keep");
    check(
      countDiscards() === 0 &&
        !container.textContent?.includes("cannot be recovered"),
      "keeping the record deleted it or stayed armed",
    );
    click("Delete selected record");
    select(5);
    check(
      !container.textContent?.includes("cannot be recovered"),
      "another record was selected while a confirmation stayed armed",
    );
    click("Delete selected record");
    click("Delete");
    await settle();
    check(
      countDiscards() === 1 && discarded[0] === keys[5],
      `deletion took the wrong record: ${JSON.stringify(discarded)}`,
    );
    await until(
      () => !!container.textContent?.includes("375 older records"),
      "inventory kept a deleted record",
    );
    check(
      !(await owner.legacyRecords()).includes(keys[5] ?? ""),
      "record survived its own deletion in storage",
    );
    check(
      (await owner.legacyRecords()).length === 375 && countCalls() === 0,
      "deletion took more than one record or sent data",
    );
    productSyncDatabase.discardLegacy = () =>
      Promise.reject(new Error("private delete error"));
    click("Delete selected record");
    click("Delete");
    await settle();
    check(
      !!container.textContent?.includes("still on this device") &&
        !container.textContent?.includes("private delete error"),
      "failed deletion claimed success or leaked its reason",
    );
    check(
      !!container.textContent?.includes("375 older records") &&
        (await owner.legacyRecords()).length === 375,
      "failed deletion dropped a record the device still holds",
    );
    const lateDelete = deferredFixture<undefined>();
    productSyncDatabase.discardLegacy = () => lateDelete.promise;
    click("Delete selected record");
    click("Delete");
    render(null);
    lateDelete.resolve(undefined);
    await settle();
    check(!container.textContent, "unmount allowed a late deletion to paint");
    tests.push(
      "deletion needs arming and one press takes only the record on screen / reselect disarms / failure keeps the record and hides its reason / unmount drains a late deletion",
    );

    productSyncDatabase.legacyRecords = async () => {
      throw new Error("private storage error");
    };
    render(createElement(ProductSyncDataNotice));
    await until(
      () => !!container.textContent?.includes("does not mean it is empty"),
      "inspection error became empty inventory",
    );
    check(
      !container.textContent?.includes("private storage error") &&
        !container.querySelector("select"),
      "failed enumeration exposed data or a partial picker",
    );
    render(null);
    const pendingKeys = deferredFixture<string[]>();
    productSyncDatabase.legacyRecords = () => pendingKeys.promise;
    render(createElement(ProductSyncDataNotice));
    flushSync(() =>
      globalThis.dispatchEvent(new Event(PRODUCT_SESSION_END_EVENT))
    );
    pendingKeys.resolve(keys);
    await settle();
    check(!container.textContent, "late inventory painted after logout");
    render(null);
    productSyncDatabase.legacyRecords = async () => [];
    render(createElement(ProductSyncDataNotice));
    await settle();
    check(!container.textContent, "empty legacy inventory showed recovery");
    tests.push(
      "enumeration failure is not absence / private errors hidden / late inventory sealed / genuinely empty inventory has no notice",
    );

    render(createElement(TelemetryBindingPanel));
    await settle();
    check(
      countCalls() === 0 && !container.textContent?.includes("Managed export"),
      "mobile About fetched Operator diagnostics while closed",
    );
    click("Export diagnostics");
    await settle();
    check(countCalls() > 0, "explicit diagnostic read missing");
    answerAll();
    await until(
      () =>
        !!container.textContent?.includes(
          "No managed export changes have been recorded",
        ),
      "plain absent state missing",
    );
    check(
      container.textContent?.includes(
        "Separately configured export may still be active",
      ) &&
        !container.textContent.includes("Service resolution writes are closed"),
      "absence became a false export failure",
    );
    click("Refresh");
    await settle();
    const pendingRead = calls.at(-1);
    click("Hide export diagnostics");
    check(
      pendingRead?.init.signal?.aborted,
      "collapsed diagnostics kept its observer",
    );
    answerAll();
    await settle();
    check(
      !container.textContent?.includes("Managed export"),
      "late diagnostics painted while closed",
    );
    tests.push(
      "mobile telemetry is lazy / GET-only Operator diagnostics / journal absence does not claim export failure / collapse seals observers",
    );

    render(null);
    const beforeDesktop = countCalls();
    render(createElement(TelemetryBindingPanel, { desktop: true }));
    await settle();
    check(
      countCalls() > beforeDesktop && button("Hide export diagnostics"),
      "desktop hid the diagnostic workbench",
    );
    for (const call of calls.slice(beforeDesktop)) {
      call.reply.resolve(
        new Response("private upstream error", { status: 503 }),
      );
    }
    await until(
      () =>
        !!container.textContent?.includes("Binding evidence is unavailable"),
      "failed telemetry read became success",
    );
    check(
      !container.textContent?.includes("private upstream error") &&
        !container.textContent?.includes("No managed export changes"),
      "failed read leaked details or became absence",
    );
    click("Refresh");
    await settle();
    const finalCount = countCalls();
    const finalRead = calls.at(-1);
    flushSync(() =>
      globalThis.dispatchEvent(new Event(PRODUCT_SESSION_END_EVENT))
    );
    check(finalRead?.init.signal?.aborted, "logout kept diagnostic request");
    answerAll();
    await settle();
    click("Export diagnostics");
    check(
      countCalls() === finalCount &&
        !container.textContent?.includes("Managed export"),
      "ended product scope reopened diagnostics",
    );
    tests.push(
      "desktop diagnostics remain visible / read failure is not absence / logout synchronously aborts and prevents reopening old scope",
    );
    return tests;
  } finally {
    render(null);
    flushSync(() => root.unmount());
    answerAll();
    await settle();
    productSyncDatabase.legacyRecords = original.legacyRecords;
    productSyncDatabase.exportLegacy = original.exportLegacy;
    productSyncDatabase.discardLegacy = original.discardLegacy;
    globalThis.fetch = original.fetch;
    URL.createObjectURL = original.createURL;
    URL.revokeObjectURL = original.revokeURL;
    HTMLAnchorElement.prototype.click = original.click;
    for (const { url } of urls) original.revokeURL(url);
    await owner.dispose();
    await native(indexedDB.deleteDatabase(dbName));
    container.remove();
  }
}
