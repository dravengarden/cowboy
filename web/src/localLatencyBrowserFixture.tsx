// Actual React Transcript, product store and IndexedDB; synthetic data only.
import type { ClientSnapshot } from "@cowboy/state-sync";
import { Transcript } from "./Transcript";
import {
  addDraft,
  editDraft,
  openSession,
  scheduleDraft,
  submitPrompt,
  unscheduleDraft,
  useStore,
} from "./store";
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import {
  productSyncDatabase,
  type ProductSyncScope,
} from "./productSyncDatabase";

const EMPTY_TIMELINE: Parameters<typeof Transcript>[0]["timeline"] = [];

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
  let releaseRestore!: () => void;
  const restore = {
    promise: new Promise<void>((resolve) => {
      releaseRestore = resolve;
    }),
    resolve: () => releaseRestore(),
  };
  let rejectWrites = false;
  let writeBarrier: Promise<void> | undefined;
  const original = productSyncDatabase.outbox;
  productSyncDatabase.outbox = <T,>(scope: ProductSyncScope) => {
    const persistence = original<T>(scope);
    return {
      ...persistence,
      load: async () => {
        if (scope.kind === "session" && scope.session === "fixture-session") {
          await restore.promise;
        }
        return await persistence.load();
      },
      save: async (value: ClientSnapshot<T>) => {
        if (scope.kind === "session") await writeBarrier;
        if (rejectWrites && scope.kind === "session") {
          throw new Error("Synthetic local storage failure");
        }
        return await persistence.save(value);
      },
    };
  };
  let snapshot!: ReturnType<typeof useStore>;
  function Probe() {
    snapshot = useStore();
    return createElement(
      "div",
      { style: { height: "600px", display: "flex", flexDirection: "column" } },
      createElement(
        "div",
        { id: "local-drafts" },
        ...(snapshot.drafts.get("fixture-session") ?? []).map((row) =>
          createElement("div", { key: row.id }, row.text)
        ),
      ),
      createElement(
        "div",
        { id: "local-queue" },
        ...(snapshot.queues.get("fixture-session") ?? []).map((row) =>
          createElement("div", { key: row.id }, row.text)
        ),
      ),
      createElement(Transcript, {
        sessionId: "fixture-session",
        timeline: snapshot.timelines.get("fixture-session") ?? EMPTY_TIMELINE,
        status: "running",
        provider: "codex",
        cwd: "/synthetic",
        loading: !snapshot.hydrated.has("fixture-session"),
        connected: snapshot.connected,
      }),
    );
  }
  const root = createRoot(document.getElementById("root")!);
  root.render(createElement(Probe));
  const samples: { kind: string; milliseconds?: number }[] = [];
  const painted = (text: string) =>
    document.getElementById("root")?.textContent?.includes(text) === true;
  try {
    await until(
      () => snapshot?.connected && snapshot.sessionsLoaded,
      "connected",
    );
    openSession("fixture-session");
    const began = performance.now();
    let settled = false;
    const outgoing = submitPrompt("fixture-session", "immediate-local-prompt")
      .then(() => {
        settled = true;
      });
    await until(
      () => painted("immediate-local-prompt"),
      "actual user bubble before restore",
    );
    const elapsed = performance.now() - began;
    if (elapsed > 200 || settled) {
      throw new Error(`Preview was gated: ${elapsed}ms`);
    }
    await delay(300);
    const before = await fetch("/fixture/metrics").then((response) =>
      response.json()
    );
    if (
      before.deliveries !== 0 || settled || !painted("immediate-local-prompt")
    ) {
      throw new Error(
        "Uncommitted preview sent, disappeared or resolved early",
      );
    }
    samples.push({ kind: "bubble-before-restoration", milliseconds: elapsed });
    restore.resolve();
    await outgoing;
    await delay(100);
    const accepted = await fetch("/fixture/metrics").then((response) =>
      response.json()
    );
    if (
      accepted.deliveries !== 1 ||
      snapshot.timelines.get("fixture-session")?.some((event) =>
        event.cmid !== undefined
      )
    ) {
      throw new Error("Receipt/echo delay not exercised");
    }
    if (!painted("immediate-local-prompt")) {
      throw new Error("Receipt hid bubble before echo");
    }
    await delay(250);
    if (!painted("immediate-local-prompt")) {
      throw new Error("Bubble vanished in receipt/echo gap");
    }
    await until(
      () =>
        snapshot.timelines.get("fixture-session")?.some((event) =>
          event.cmid !== undefined
        ) === true,
      "echo",
    );
    await until(() => painted("immediate-local-prompt"), "echo rendered");
    samples.push({ kind: "receipt-retains-bubble-until-echo" });

    const beforeSaved = async (
      action: () => Promise<void>,
      visible: () => boolean,
      label: string,
    ) => {
      let releaseWrite!: () => void;
      writeBarrier = new Promise<void>((resolve) => {
        releaseWrite = resolve;
      });
      let completed = false;
      const saving = action().then(() => {
        completed = true;
      });
      try {
        await until(visible, label);
        if (completed) {
          throw new Error(`${label} did not exercise the local write barrier`);
        }
        releaseWrite();
        await saving;
      } finally {
        releaseWrite();
        writeBarrier = undefined;
      }
    };
    await beforeSaved(
      () => addDraft("fixture-session", "local-draft", []),
      () => painted("local-draft"),
      "draft before save",
    );
    const draft = snapshot.drafts.get("fixture-session")?.find((row) =>
      row.text === "local-draft"
    );
    if (!draft) throw new Error("Missing draft");
    await beforeSaved(
      () => editDraft("fixture-session", draft.id, "edited-local-draft", []),
      () => painted("edited-local-draft"),
      "edit before save",
    );
    const fireAtMs = Date.now() + 60000;
    await beforeSaved(
      () => scheduleDraft("fixture-session", { id: draft.id, fireAtMs }),
      () =>
        snapshot.drafts.get("fixture-session")?.find((row) =>
          row.id === draft.id
        )?.schedule?.fire_at_ms === fireAtMs,
      "schedule before save",
    );
    await beforeSaved(
      () => unscheduleDraft("fixture-session", draft.id),
      () =>
        snapshot.drafts.get("fixture-session")?.find((row) =>
          row.id === draft.id
        )?.schedule === undefined,
      "cancel schedule before save",
    );
    samples.push({ kind: "draft-add-edit-schedule-cancel-before-save" });

    rejectWrites = true;
    let rejected = false;
    try {
      await submitPrompt("fixture-session", "unsaved-local-prompt");
    } catch {
      rejected = true;
    }
    if (!rejected) throw new Error("Failed storage reported success");
    await until(
      () => !painted("unsaved-local-prompt"),
      "failed preview rollback",
    );
    const after = await fetch("/fixture/metrics").then((response) =>
      response.json()
    );
    if (after.deliveries !== 1) {
      throw new Error("Unsaved prompt reached transport");
    }
    rejectWrites = false;
    samples.push({ kind: "failed-save-rolls-back-without-sending" });
    return samples;
  } finally {
    restore.resolve();
    rejectWrites = false;
    root.unmount();
    globalThis.dispatchEvent(new Event("cowboy:product-sign-out"));
  }
}
