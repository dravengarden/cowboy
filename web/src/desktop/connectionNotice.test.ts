import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { connectionNotice } from "./connectionNotice.ts";
import type { SyncStatus } from "../syncStatus.ts";

const now = 1_000_000;

function status(overrides: Partial<SyncStatus>): SyncStatus {
  return {
    phase: "live",
    since: now - 60_000,
    outbox: { pending: 0, held: 0, sessions: [] },
    updateReady: false,
    ...overrides,
  };
}

test("an unanswered server names the thing to check and when it retries", () => {
  const notice = connectionNotice(
    status({
      phase: "unreachable",
      retryAt: now + 4_200,
      lastLiveAt: now - 120_000,
      outbox: { pending: 2, held: 0, sessions: ["a"] },
    }),
    "unreachable",
    now,
  );
  assertEquals(notice, {
    tone: "warning",
    title: "Can't reach Cowboy server",
    hint:
      "The network is up but the server is not answering. Check the VPN or the server.",
    meta:
      "Retrying in 5 s · last synced 2 min ago · 2 messages will send automatically",
    countdown: "Retrying in 5 s",
    canRetry: true,
  });
});

test("a device without network points at its own network", () => {
  const notice = connectionNotice(status({ phase: "offline" }), "offline", now);
  assertEquals(notice?.title, "Offline");
  assertEquals(
    notice?.hint,
    "This device has no network. Check Wi-Fi or Ethernet.",
  );
  assertEquals(
    notice?.meta,
    "Messages you write are saved and send automatically",
  );
});

test("blips, live sessions and decision phases leave the strip empty", () => {
  // Not yet past the presentation debounce.
  assertEquals(
    connectionNotice(status({ phase: "unreachable" }), null, now),
    null,
  );
  // A short reconnect stays in the status line.
  assertEquals(
    connectionNotice(status({ phase: "connecting" }), "connecting", now),
    null,
  );
  assertEquals(connectionNotice(status({ phase: "live" }), null, now), null);
  // Sign-in and account fences already own a decision surface.
  assertEquals(
    connectionNotice(status({ phase: "auth_required" }), "auth_required", now),
    null,
  );
  assertEquals(
    connectionNotice(status({ phase: "fenced" }), "fenced", now),
    null,
  );
});

test("recovery flashes a quiet confirmation without actions", () => {
  assertEquals(connectionNotice(status({ phase: "live" }), "recovered", now), {
    tone: "success",
    title: "Reconnected",
    hint: null,
    meta: null,
    countdown: null,
    canRetry: false,
  });
});
