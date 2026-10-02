// Real product store, IndexedDB and WebSocket; synthetic content and delays only.
// oxlint-disable promise/avoid-new
import { createElement } from "react";
import type { ClientSnapshot, LocalPersistence } from "@cowboy/state-sync";
import { createRoot } from "react-dom/client";
import { bindProductSyncPrincipal } from "./productSyncIdentity.ts";
import { productSyncDatabase, type ProductSyncScope } from "./productSyncDatabase.ts";
import { TranscriptCachedCaption } from "./TranscriptCachedCaption.tsx";
import { Transcript } from "./Transcript.tsx";
import { activateDraft, openSession, submitPrompt, useStore } from "./store.ts";
import { promptEchoReadyToReplaceOptimistic } from "./sendImagePreviews.ts";
import type { Attachment } from "./attachments.ts";
import { SessionObligationBadge } from "./SessionOfflineBadges.tsx";
import { optimisticQuestionKey } from "./explore/optimisticPages.ts";

const delay = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));
let snapshot: ReturnType<typeof useStore>;
function Probe() {
  snapshot = useStore();
  const scenario = new URL(location.href).searchParams.get("scenario") ?? "";
  if (scenario === "continuity" || scenario.startsWith("failed-recovery")) {
    const transcript = createElement(Transcript, {
      sessionId: "fixture-session",
      timeline: snapshot.timelines.get("fixture-session") ?? [],
      status: "running",
      provider: "codex",
      cwd: "/synthetic",
      loading: false,
      connected: snapshot.connected,
      historyPaging: "page",
      ...(scenario.startsWith("failed-recovery") ? {
        liveTail: false,
        visibleItemKeys: new Set([
          ...(snapshot.optimisticMessages.get("fixture-session") ?? [])
            .filter((row) => row.text === "new working prompt")
            .map(optimisticQuestionKey),
          ...(snapshot.timelines.get("fixture-session") ?? []).map((event) => String(event.seq)),
        ]),
      } : {}),
    });
    return scenario === "continuity" ? transcript : createElement("div", null,
      createElement(SessionObligationBadge, { sessionId: "fixture-session" }),
      createElement("div", { id: "recovery-drafts" }, ...(snapshot.drafts.get("fixture-session") ?? []).map((row) =>
        createElement("div", { key: row.id }, `${row.text}:${row.attachments.length}`))), transcript);
  }
  return createElement(TranscriptCachedCaption, {
    sessionId: "fixture-session",
  });
}
async function until(predicate: () => boolean, label: string, timeout = 8000) {
  const started = performance.now();
  while (!predicate()) {
    if (performance.now() - started > timeout) {
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
  if (scenario === "update-settings") {
    const { runClientUpdateFixture } = await import("./clientUpdateBrowserFixture.tsx");
    return await runClientUpdateFixture();
  }
  if (scenario === "failed-recovery-storage-error") {
    const outbox = productSyncDatabase.outbox.bind(productSyncDatabase);
    productSyncDatabase.outbox = <T>(scope: ProductSyncScope): LocalPersistence<ClientSnapshot<T>> => {
      const persistence = outbox<T>(scope);
      return { ...persistence, save: async (value) => {
        if (JSON.stringify(value).includes('"name":"addDraft"')) throw new Error("Synthetic draft recovery save failure");
        await persistence.save(value);
      } };
    };
  }
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
    if (scenario.startsWith("failed-recovery")) {
      openSession(session);
      await until(() => snapshot?.connected && snapshot.hydrated.has(session), "connected session");
      const data = "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
      const image: Attachment = { id: "old-shot", name: "old.gif", isImage: true, mimeType: "image/gif",
        previewUrl: `data:image/gif;base64,${data}`, block: { type: "image", data, mimeType: "image/gif" } };
      await submitPrompt(session, "older unconfirmed caption", [image]);
      await until(() => snapshot.optimisticMessages.get(session)?.some((row) => row.status === "failed") === true, "failed older send retained");
      const oldCmid = snapshot.optimisticMessages.get(session)![0]!.cmid;
      await until(() => document.querySelector('[aria-label="1 message needs attention"]') !== null, "failure badge paints");
      let heldBubbleDisappeared = false;
      const heldBubbleObserver = new MutationObserver(() => {
        if (document.querySelector(`[data-key="opt-${oldCmid}"]`) === null) heldBubbleDisappeared = true;
      });
      heldBubbleObserver.observe(document.body, { childList: true, subtree: true });
      await submitPrompt(session, "new working prompt");
      await until(() => document.querySelector(`[data-key="opt-${oldCmid}"]`) !== null &&
        document.body.textContent?.includes("new working prompt") === true,
        "older held bubble remains visible beside a newer page's pending prompt");
      await until(() => JSON.stringify(snapshot.timelines.get(session) ?? []).includes("new working prompt"), "new user echo");
      heldBubbleObserver.disconnect();
      if (heldBubbleDisappeared) throw new Error("changing pages briefly hid the older unconfirmed message");
      if (!snapshot.optimisticMessages.get(session)?.some((row) => row.cmid === oldCmid)) throw new Error("older send retired before agent progress");
      if (scenario === "failed-recovery-no-work") {
        await until(() => snapshot.timelines.get(session)?.some((event) => event.kind === "turn_end") === true, "turn ends before work");
        if (!snapshot.optimisticMessages.get(session)?.some((row) => row.cmid === oldCmid && row.status === "failed") || (snapshot.drafts.get(session) ?? []).length !== 0) throw new Error("a turn without work retired the failed message");
        return ["receipt and user echo without agent work preserve the held message"];
      }
      await until(() => document.body.textContent?.includes("Agent is working again") === true, "actual resumed work");
      if (scenario === "failed-recovery-storage-error") {
        await delay(300);
        if (!snapshot.optimisticMessages.get(session)?.some((row) => row.cmid === oldCmid && row.status === "failed")) throw new Error("failed draft save lost the source message");
        return ["draft save failure preserves the original held message and image"];
      }
      await until(() => (snapshot.optimisticMessages.get(session) ?? []).length === 0, "old bottom error retired");
      await until(() => document.querySelector('[aria-label="1 message needs attention"]') === null, "attention badge clears");
      await until(() => snapshot.drafts.get(session)?.some((row) => row.text === "older unconfirmed caption" && row.attachments.length === 1 && row.status === undefined) === true, "old content saved as acknowledged draft");
      if (!await retained("older unconfirmed caption")) throw new Error("recovered draft not durable");
      return ["new prompt must start agent work before old errors clear", "caption and image retained as a durable draft", "old delivery never resubmitted"];
    }
    if (scenario === "continuity") {
      openSession(session);
      await until(() => document.querySelector('[data-key="1"] img') !== null, "earlier image paints");
      const previous = document.querySelector('[data-key="1"]')!;
      const failures: string[] = [];
      let currentText = "";
      let currentImages = 0;
      const check = () => {
        if (!previous.isConnected || document.querySelector('[data-key="1"]') !== previous) {
          failures.push("previous image removed or remounted");
        }
        if (currentText && !document.body.textContent?.includes(currentText)) failures.push("current caption disappeared");
        if (document.querySelectorAll("img").length < 1 + currentImages) failures.push("current attachments disappeared");
      };
      const observer = new MutationObserver(check);
      observer.observe(document.getElementById("root")!, { childList: true, subtree: true });
      try {
        const data = "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
        const image: Attachment = { id: "shot", name: "shot.gif", isImage: true, mimeType: "image/gif", previewUrl: `data:image/gif;base64,${data}`, block: { type: "image", data, mimeType: "image/gif" } };
        const file: Attachment = { id: "file", name: "notes.txt", isImage: false, mimeType: "text/plain", block: { type: "resource_link", uri: "file:///notes.txt", name: "notes.txt" } };
        const cases: [string, Attachment[]][] = [
          ["first new text", []], ["second new text", []],
          ["two image caption", [image, { ...image, id: "shot2", name: "shot2.gif" }]],
          ["file caption", [file]],
        ];
        for (const [text, attachments] of cases) {
          currentText = "";
          currentImages = 0;
          const previousImageCount = document.querySelectorAll("img").length - 1;
          await submitPrompt(session, text, attachments);
          await until(() => document.body.textContent?.includes(text) === true, "local send immediately visible");
          currentText = text;
          currentImages = previousImageCount + attachments.filter((attachment) => attachment.isImage).length;
          const untilEcho = performance.now() + 500;
          while (performance.now() < untilEcho) {
            check();
            if (!document.body.textContent?.includes(text)) failures.push("new text disappeared before echo");
            await delay(10);
          }
          await until(() => (snapshot.optimisticMessages.get(session) ?? []).length === 0, "echo replaces local copy");
          await delay(50);
          check();
          if (!document.body.textContent?.includes(text)) failures.push("new text missing after echo");
        }
      } finally {
        observer.disconnect();
      }
      if (failures.length) throw new Error(failures.join(", "));
      return ["earlier image node retained through four sends", "text, two images with caption, and file stay visible through delayed multipart echoes"];
    }
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
    if (scenario === "lost-send") {
      openSession(session);
      await submitPrompt(session, "retained through half-open socket");
      await until(
        () => (snapshot.optimisticMessages.get(session) ?? []).length === 0,
        "early connection check replays durable message",
        25_000,
      );
      const echoes = (snapshot.timelines.get(session) ?? []).filter((event) =>
        event.kind === "update" &&
        event.update.sessionUpdate === "user_message_chunk"
      );
      if (echoes.length !== 1) {
        throw new Error("lost send recovery duplicated or lost the prompt");
      }
      return [
        "quiet half-open socket checked early",
        "same durable message replayed",
        "one authoritative echo",
      ];
    }
    if (scenario === "draft-send") {
      openSession(session);
      await until(
        () => (snapshot.drafts.get(session) ?? []).length === 1,
        "image draft hydration",
      );
      const sending = activateDraft(session, "fixture-draft");
      await until(
        () => (snapshot.optimisticMessages.get(session) ?? []).length === 1,
        "optimistic draft send",
      );
      const message = snapshot.optimisticMessages.get(session)![0]!;
      if (
        message.attachments.length !== 2 || message.cmid === "draft-creation"
      ) {
        throw new Error("draft send lost its images or operation identity");
      }
      await sending;
      await until(
        () =>
          (snapshot.timelines.get(session) ?? []).filter((event) =>
            event.kind === "update" &&
            event.update.sessionUpdate === "user_message_chunk"
          ).length === 3,
        "complete image echo",
      );
      await until(
        () => (snapshot.optimisticMessages.get(session) ?? []).length === 0,
        "Sending cleared",
      );
      if (
        !promptEchoReadyToReplaceOptimistic(
          message,
          snapshot.timelines.get(session) ?? [],
        )
      ) {
        throw new Error("confirmed echo cannot replace the image preview");
      }
      return [
        "two-image draft retained until complete echo",
        "Sending cleared on exact operation identity",
      ];
    }
    if (scenario === "transcript-recovery" || scenario === "slow-mobile") {
      await productSyncDatabase.cache({
        kind: "session",
        session,
        state: "tail",
      }).save({
        receivedAt: Date.now() - 17 * 60_000,
        lastSeq: 1,
        reachedStart: true,
        events: [{
          session_id: session,
          seq: 1,
          kind: "update",
          update: {
            sessionUpdate: "agent_message_chunk",
            content: { type: "text", text: "old partial answer" },
          },
        }],
      });
      openSession(session);
      await until(
        () => snapshot.transcriptSources.get(session)?.source === "replica",
        "cached paint",
      );
      if (scenario === "slow-mobile") {
        await submitPrompt(session, "slow mobile prompt");
        await delay(11_000);
        const outgoing = snapshot.optimisticMessages.get(session) ?? [];
        if (outgoing.length !== 1 || outgoing[0]?.status !== "sending") {
          throw new Error("slow confirmation was prematurely marked failed");
        }
        await until(
          () => snapshot.transcriptSources.get(session)?.source === "live",
          "12-second bootstrap completes without repeated abortion",
          8_000,
        );
        if (document.querySelector("[data-transcript-cached-caption]")) {
          throw new Error("slow bootstrap left the cached sync caption stuck");
        }
        await until(
          () => (snapshot.optimisticMessages.get(session) ?? []).length === 0,
          "33-second authoritative echo retires the retained prompt",
          28_000,
        );
        return [
          "slow transcript completes",
          "no premature 10-second failure",
          "late echo retires local copy",
        ];
      }
      await until(
        () => snapshot.transcriptSources.get(session)?.syncState === "retrying",
        "failed bootstrap retries cached tail",
      );
      if (!document.body.textContent?.includes("Retry sync")) {
        throw new Error("retry action missing");
      }
      await until(
        () => snapshot.transcriptSources.get(session)?.source === "live",
        "authoritative snapshot after repeated failures",
        20_000,
      );
      const rows = snapshot.timelines.get(session) ?? [];
      if (
        rows.length !== 1 ||
        !JSON.stringify(rows).includes("fresh complete answer")
      ) {
        throw new Error(
          "canonical snapshot did not replace stale partial answer exactly once",
        );
      }
      if (document.querySelector("[data-transcript-cached-caption]")) {
        throw new Error("cached caption survived live acknowledgement");
      }
      return [
        "replica paint",
        "503 retry",
        "empty 200 not acknowledgement",
        "automatic recovery",
        "no duplicates",
        "caption cleared",
      ];
    }
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
