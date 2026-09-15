// Real product store, IndexedDB and WebSocket; synthetic content and delays only.
// oxlint-disable promise/avoid-new
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import { bindProductSyncPrincipal } from "./productSyncIdentity.ts";
import { productSyncDatabase } from "./productSyncDatabase.ts";
import { openSession, submitPrompt, useStore } from "./store.ts";

const delay = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));
let snapshot: ReturnType<typeof useStore>;
function Probe() {
  snapshot = useStore();
  return null;
}
async function until(predicate: () => boolean, label: string) {
  const started = performance.now();
  while (!predicate()) {
    if (performance.now() - started > 8000) {
      throw new Error(
        `fixture timed out: ${label}; ${
          JSON.stringify({
            connected: snapshot?.connected,
            error: snapshot?.lastError,
            timeline: snapshot?.timelines.get("fixture-session"),
          })
        }`,
      );
    }
    await delay(5);
  }
}

async function retained(text: string): Promise<boolean> {
  // Inspect the durable record independently after the product owner is sealed.
  return await new Promise<boolean>((resolve, reject) => {
    const request = indexedDB.open("shared-utils-sync", 2);
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const db = request.result;
      const transaction = db.transaction("clients", "readonly");
      const rows = transaction.objectStore("clients").getAll();
      transaction.oncomplete = () => {
        db.close();
        resolve(JSON.stringify(rows.result).includes(text));
      };
      transaction.onabort = () => {
        db.close();
        reject(transaction.error);
      };
    };
  });
}

export async function run() {
  const scenario = new URL(location.href).searchParams.get("scenario") ??
    "timing";
  bindProductSyncPrincipal("fixture-user");
  const queues = productSyncDatabase.queueSessions.bind(productSyncDatabase);
  productSyncDatabase.queueSessions = async () => {
    // Model an unrelated slow cache enumeration, without delaying the authored
    // message's own durable transaction or replacing IndexedDB with a mock.
    await delay(500);
    return await queues();
  };
  const session = "fixture-session";
  const root = createRoot(document.getElementById("root")!);
  const started = performance.now();
  root.render(createElement(Probe));
  const samples: { kind: string; milliseconds: number }[] = [];
  const send = async (kind: string, began: number) => {
    const text = `synthetic-${crypto.randomUUID()}`;
    await submitPrompt(session, text);
    // The submit promise only promises local visibility. Measure the actual
    // server echo, never a hidden spinner or an optimistic bubble.
    await until(
      () =>
        (snapshot.timelines.get(session) ?? []).some((event) =>
          event.kind === "update" &&
          event.update.sessionUpdate === "user_message_chunk" &&
          (event.update.content as { text?: string })?.text === text
        ),
      `echo ${kind} ${text}`,
    );
    samples.push({ kind, milliseconds: performance.now() - began });
  };
  try {
    if (scenario === "missing-protocol") {
      await until(() => snapshot !== undefined, "mounted product subscriber");
      const text = `retained-${crypto.randomUUID()}`;
      await submitPrompt(session, text);
      // Allow the server to open and send bootstrap frames without selecting
      // our subprotocol. Neither those frames nor pending data may be admitted.
      await delay(700);
      if (
        snapshot.connected || snapshot.sessionsLoaded || !await retained(text)
      ) {
        throw new Error("unnegotiated socket admitted data or lost the outbox");
      }
      return ["unnegotiated socket rejected", "durable prompt retained"];
    }
    await until(
      () => snapshot?.connected && snapshot.sessionsLoaded,
      "initial connection",
    );
    openSession(session);
    if (scenario === "changed-dataset") {
      await fetch("/fixture/switch-dataset", { method: "POST" });
      globalThis.dispatchEvent(new Event("online"));
      await until(() => !snapshot.connected, "disconnect before replacement");
      const text = `retained-${crypto.randomUUID()}`;
      await submitPrompt(session, text);
      await until(
        () =>
          snapshot.lastError?.message.includes("Service dataset changed") ??
            false,
        "replacement fenced",
      );
      await until(
        () => productSyncDatabase.lifecycle.phase === "disposed",
        "owner drained",
      );
      await fetch("/fixture/restore-dataset", { method: "POST" });
      globalThis.dispatchEvent(new Event("online"));
      await delay(700);
      if (snapshot.connected || !await retained(text)) {
        throw new Error("replacement revived the owner or lost the outbox");
      }
      return [
        "changed dataset rejected",
        "ABA owner remained sealed",
        "durable prompt retained",
      ];
    }
    await send("cold", started);
    for (let index = 0; index < 8; index++) {
      await send("warm", performance.now());
    }
    for (let index = 0; index < 8; index++) {
      const began = performance.now();
      globalThis.dispatchEvent(new Event("online"));
      await until(() => !snapshot.connected, "disconnect");
      await until(() => snapshot.connected, "reconnect");
      await send("reconnect", began);
    }
    return samples;
  } finally {
    root.unmount();
    globalThis.dispatchEvent(new Event("cowboy:product-sign-out"));
  }
}
