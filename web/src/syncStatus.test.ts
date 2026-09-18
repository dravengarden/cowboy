import { assertEquals, assertStrictEquals } from "jsr:@std/assert";
import {
  deriveSyncPhase,
  deriveSyncStatus,
  ordinal,
  presentedSyncPhase,
  relativeAge,
  SYNC_PRESENTATION_DEBOUNCE_MS,
  SYNC_RECOVERED_FLASH_MS,
  syncStatusLabel,
  type SyncStatusInput,
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

Deno.test("deriveSyncPhase orders fences, auth, liveness, capacity and outages", () => {
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
    "offline",
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

Deno.test("deriveSyncStatus keeps `since` across same-phase updates and preserves identity", () => {
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

Deno.test("labels are compact and count what matters", () => {
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

Deno.test("presentedSyncPhase debounces blips and flashes recovery once", () => {
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
