// Cold metadata gestures against the actual product store and IndexedDB.
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import {
  productSyncDatabase,
  type ProductSyncScope,
} from "./productSyncDatabase";
const delay = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));
async function until(predicate: () => boolean, label: string) {
  const began = performance.now();
  while (!predicate()) {
    if (performance.now() - began > 8000) {
      throw new Error(`Timed out: ${label}`);
    }
    await delay(5);
  }
}
export async function run() {
  bindProductSyncPrincipal("fixture-user");
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const original = productSyncDatabase.outbox;
  productSyncDatabase.outbox = <T,>(scope: ProductSyncScope) => {
    const persistence = original<T>(scope);
    return {
      ...persistence,
      load: async () => {
        await gate;
        return await persistence.load();
      },
    };
  };
  const {
    useStore,
    renameSession,
    reorderSessions,
    createSessionFolder,
    renameSessionFolder,
  } = await import("./store");
  let snapshot!: ReturnType<typeof useStore>;
  function Probe() {
    snapshot = useStore();
    return createElement(
      "div",
      null,
      ...snapshot.sessions.map((session) =>
        createElement(
          "div",
          { key: session.id, "data-session": session.id },
          session.title,
        )
      ),
      ...snapshot.sessionFolders.folders.map((folder) =>
        createElement("div", { key: folder.id }, folder.name)
      ),
    );
  }
  const root = createRoot(document.getElementById("root")!);
  root.render(createElement(Probe));
  try {
    await until(
      () => snapshot?.sessionsLoaded && snapshot.sessions.length === 2,
      "session index",
    );
    const began = performance.now();
    renameSession("fixture-session", "instant-local-title");
    reorderSessions(["fixture-other", "fixture-session"]);
    const folder = createSessionFolder("instant-local-folder", null);
    if (!folder) throw new Error("Folder not created");
    renameSessionFolder(folder, "renamed-local-folder");
    await until(
      () =>
        document.getElementById("root")?.textContent?.includes(
            "instant-local-title",
          ) === true &&
        document.getElementById("root")?.textContent?.includes(
            "renamed-local-folder",
          ) === true &&
        document.querySelector("[data-session]")?.getAttribute(
            "data-session",
          ) === "fixture-other",
      "local metadata paint",
    );
    const elapsed = performance.now() - began;
    const before = await fetch("/fixture/metrics").then((response) =>
      response.json()
    );
    if (elapsed > 200 || before.metadataMutations !== 0) {
      throw new Error("Metadata preview waited or bypassed storage");
    }
    release();
    await until(
      () => snapshot.sessions[0]?.id === "fixture-other",
      "stable order",
    );
    for (let attempt = 0; attempt < 100; attempt++) {
      const after = await fetch("/fixture/metrics").then((response) =>
        response.json()
      );
      if (after.metadataMutations === 4) {
        return [{
          kind: "rename-order-folder-before-restoration",
          milliseconds: elapsed,
        }];
      }
      await delay(10);
    }
    throw new Error("Metadata was not durably handed to transport");
  } finally {
    release();
    root.unmount();
    globalThis.dispatchEvent(new Event("cowboy:product-sign-out"));
  }
}
