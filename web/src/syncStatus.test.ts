import { test } from "bun:test";
import { assert, assertEquals, assertStrictEquals } from "@std/assert";
import {
  attentionCount,
  deriveSyncPhase,
  deriveSyncStatus,
  ordinal,
  presentedSyncPhase,
  relativeAge,
  SYNC_PRESENTATION_DEBOUNCE_MS,
  SYNC_RECOVERED_FLASH_MS,
  syncStatusDetail,
  syncStatusLabel,
  type SyncStatusInput,
  syncStatusTone,
  withHeld,
} from "./syncStatus.ts";

const base: SyncStatusInput = {
  connected: true,
  socket: "open",
  online: true,
  attempts: 0,
  pausedForAuth: false,
  fenced: false,
  silenceMs: 0,
  outbox: { pending: 0, held: 0, sessions: [] },
  updateReady: false,
};

test("deriveSyncPhase orders fences, auth, liveness, capacity and outages", () => {
  assertEquals(deriveSyncPhase(base), "live");
  assertEquals(deriveSyncPhase({ ...base, silenceMs: 31_000 }), "degraded");
  assertEquals(deriveSyncPhase({ ...base, fenced: true }), "fenced");
  assertEquals(deriveSyncPhase({ ...base, pausedForAuth: true }), "auth_required");
  assertEquals(
    deriveSyncPhase({ ...base, connected: false, socket: "connecting", attempts: 1 }),
    "connecting",
  );
  assertEquals(
    deriveSyncPhase({ ...base, connected: false, socket: "none", attempts: 2 }),
    "unreachable",
  );
  assertEquals(
    deriveSyncPhase({ ...base, connected: false, socket: "none", online: false }),
    "offline",
  );
  assertEquals(
    deriveSyncPhase({ ...base, connected: false, socket: "none", capacity: "waiting" }),
    "waiting",
  );
  // An admitted socket outranks a stale capacity notice.
  assertEquals(deriveSyncPhase({ ...base, capacity: "waiting" }), "live");
});

test("deriveSyncStatus keeps `since` across same-phase updates and preserves identity", () => {
  const first = deriveSyncStatus({ ...base, connected: false, socket: "connecting", attempts: 1 }, undefined, 1000);
  assertEquals(first.phase, "connecting");
  assertEquals(first.since, 1000);
  const second = deriveSyncStatus(
    { ...base, connected: false, socket: "connecting", attempts: 1 },
    first,
    5000,
  );
  assertStrictEquals(second, first);
  const withRetry = deriveSyncStatus(
    { ...base, connected: false, socket: "none", attempts: 1, retryAt: 9000 },
    second,
    6000,
  );
  assertEquals(withRetry.since, 1000);
  assertEquals(withRetry.retryAt, 9000);
  const live = deriveSyncStatus(base, withRetry, 7000);
  assertEquals(live.phase, "live");
  assertEquals(live.since, 7000);
  assertEquals(live.retryAt, undefined);
});

test("labels are compact and count what matters", () => {
  const now = 10_000;
  const offline = deriveSyncStatus(
    { ...base, connected: false, socket: "none", online: false, outbox: { pending: 2, held: 0, sessions: ["a"] } },
    undefined,
    now,
  );
  assertEquals(syncStatusLabel(offline, now), "Offline · 2 queued");
  const waiting = deriveSyncStatus(
    { ...base, connected: false, socket: "none", capacity: "waiting", position: 2 },
    undefined,
    now,
  );
  assertEquals(syncStatusLabel(waiting, now), "Waiting for a seat (2nd)");
  const retry = deriveSyncStatus(
    { ...base, connected: false, socket: "none", attempts: 1, retryAt: now + 4_200 },
    undefined,
    now,
  );
  assertEquals(syncStatusLabel(retry, now), "Reconnecting in 5 s");
  const attention = deriveSyncStatus(
    { ...base, outbox: { pending: 1, held: 1, sessions: ["a"] } },
    undefined,
    now,
  );
  assertEquals(syncStatusLabel(attention, now), "1 message needs attention");
  assertEquals(syncStatusLabel(deriveSyncStatus(base, undefined, now), now), null);
  assertEquals(ordinal(1), "1st");
  assertEquals(ordinal(12), "12th");
  assertEquals(ordinal(23), "23rd");
  assertEquals(relativeAge(now - 10_000, now), "just now");
  assertEquals(relativeAge(now - 180_000, now), "3 min ago");
  assertEquals(relativeAge(now - 3_600_000, now), "1 hour ago");
  assertEquals(relativeAge(undefined, now), null);
});

test("presentedSyncPhase debounces blips and flashes recovery once", () => {
  const start = 1000;
  const connecting = deriveSyncStatus(
    { ...base, connected: false, socket: "connecting", attempts: 1 },
    undefined,
    start,
  );
  assertEquals(presentedSyncPhase(connecting, null, undefined, start + 500), null);
  assertEquals(
    presentedSyncPhase(connecting, null, undefined, start + SYNC_PRESENTATION_DEBOUNCE_MS),
    "connecting",
  );
  // Once shown, a same-phase status stays shown without waiting again.
  assertEquals(presentedSyncPhase(connecting, "connecting", undefined, start + 100), "connecting");
  const live = deriveSyncStatus(base, connecting, start + 5000);
  assertEquals(presentedSyncPhase(live, "connecting", start + 5000, start + 5100), "recovered");
  assertEquals(
    presentedSyncPhase(live, "connecting", start + 5000, start + 5000 + SYNC_RECOVERED_FLASH_MS),
    null,
  );
  const held = deriveSyncStatus({ ...base, outbox: { pending: 0, held: 1, sessions: ["a"] } }, live, start + 9000);
  assertEquals(presentedSyncPhase(held, null, undefined, start + 9000), "live");
});

test("attention counts held rows outside the opened session that were not dismissed", () => {
  const sessions = [
    { id: "a", ids: ["m1", "m2"] },
    { id: "b", ids: ["m3"] },
  ];
  assertEquals(attentionCount(sessions, "a", new Set()), 1);
  assertEquals(attentionCount(sessions, "c", new Set()), 3);
  assertEquals(attentionCount(sessions, null, new Set(["m1", "m3"])), 1);
  assertEquals(attentionCount(sessions, "b", new Set(["m1", "m2"])), 0);
  assertEquals(attentionCount([], "a", new Set()), 0);
});

test("withHeld keeps identity when the held count is unchanged", () => {
  const status = deriveSyncStatus(base, undefined, 1_000);
  assertStrictEquals(withHeld(status, status.outbox.held), status);
  const adjusted = withHeld(status, 2);
  assertEquals(adjusted.outbox.held, 2);
  assertEquals(adjusted.phase, status.phase);
});

test("an unanswered server is named apart from a device without network", () => {
  const now = 10_000;
  const unreachable = deriveSyncStatus(
    {
      ...base,
      connected: false,
      socket: "none",
      attempts: 3,
      retryAt: now + 4_000,
      outbox: { pending: 1, held: 0, sessions: ["a"] },
    },
    undefined,
    now,
  );
  assertEquals(unreachable.phase, "unreachable");
  // The countdown survives so the surface can say when the next try is.
  assertEquals(unreachable.retryAt, now + 4_000);
  assertEquals(syncStatusLabel(unreachable, now), "Can't reach Cowboy · 1 queued");
  assert(syncStatusDetail(unreachable, now).includes("server is not answering"));
  assertEquals(syncStatusTone("unreachable"), "warning");

  // No network outranks attempt counting: the device is the thing to check.
  const offline = deriveSyncStatus(
    { ...base, connected: false, socket: "none", online: false, attempts: 5 },
    undefined,
    now,
  );
  assertEquals(offline.phase, "offline");
  assert(syncStatusDetail(offline, now).includes("check its network"));
});
