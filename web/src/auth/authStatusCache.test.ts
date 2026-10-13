import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import type { AuthStatus } from "./authApi.ts";
import {
  authStatusCacheExpiry,
  bootAuthDecision,
  cachedAuthDecision,
  forgetAuthStatus,
  readCachedAuthStatus,
  readStaleAuthStatus,
  rememberAuthStatus,
} from "./authStatus.ts";

function memoryStorage(): Pick<Storage, "getItem" | "setItem" | "removeItem"> {
  const data = new Map<string, string>();
  return {
    getItem: (key) => data.get(key) ?? null,
    setItem: (key, value) => {
      data.set(key, value);
    },
    removeItem: (key) => {
      data.delete(key);
    },
  };
}

const registration = { accepts_registration: false, mode: "closed" } as unknown as AuthStatus["registration"];

function status(me: AuthStatus["me"], session?: AuthStatus["session"]): AuthStatus {
  return { registration, ...(session !== undefined ? { session } : {}), ...(me !== undefined ? { me } : {}) };
}

test("a cached principal mounts only for retry probes and only before its deadline", () => {
  const storage = memoryStorage();
  const now = 1_000_000;
  const me = { account: "draven", user_id: "user-a", role: "owner" as const, primary_reauth_due_at_ms: now + 5_000 };
  rememberAuthStatus(status(me, { idle_timeout_ms: 60_000 } as AuthStatus["session"]), now, storage);
  assertEquals(readCachedAuthStatus(now + 1_000, storage)?.me.user_id, "user-a");
  assertEquals(cachedAuthDecision({ view: "retry" }, now + 1_000, storage), {
    view: "ready",
    me,
    cached: true,
  });
  assertEquals(cachedAuthDecision({ view: "login" }, now + 1_000, storage), null);
  assertEquals(cachedAuthDecision({ view: "activating" }, now + 1_000, storage), null);
  // The primary-login deadline is earlier than the idle window and wins.
  assertEquals(readCachedAuthStatus(now + 5_000, storage), null);
  forgetAuthStatus(storage);
  assertEquals(readCachedAuthStatus(now, storage), null);
});

test("expiry follows the earliest known deadline and auth-off never expires", () => {
  const now = 10;
  assertEquals(
    authStatusCacheExpiry(status({ account: "a", user_id: "u", role: "owner", auth_enabled: false }), now),
    null,
  );
  assertEquals(authStatusCacheExpiry(status(undefined), now), now);
  assertEquals(
    authStatusCacheExpiry(
      status({ account: "a", user_id: "u", role: "owner", passkey_reauth_required: true, passkey_reauth_due_at_ms: 40 }, {
        idle_timeout_ms: 100,
      } as AuthStatus["session"]),
      now,
    ),
    40,
  );
});

test("a status without a user id clears the cache instead of storing a label", () => {
  const storage = memoryStorage();
  rememberAuthStatus(status({ account: "a", user_id: "u", role: "owner" }), 1, storage);
  rememberAuthStatus(status({ account: "legacy", role: "owner" }), 2, storage);
  assertEquals(readCachedAuthStatus(3, storage), null);
  storage.setItem("cowboy:auth-status-cache", "{not json");
  assertEquals(readCachedAuthStatus(3, storage), null);
});

test("the app boots from a still-valid cached principal before any probe answers", () => {
  const storage = memoryStorage();
  const now = 2_000_000;
  assertEquals(bootAuthDecision(now, storage), null);
  const me = { account: "draven", user_id: "user-a", role: "owner" as const };
  rememberAuthStatus(status(me, { idle_timeout_ms: 60_000 } as AuthStatus["session"]), now, storage);
  assertEquals(bootAuthDecision(now + 1_000, storage), { view: "ready", me, cached: true });
  // Past the earliest server deadline an online device waits for the server
  // again; a device that knows it is offline opens on the stale principal.
  assertEquals(bootAuthDecision(now + 60_001, storage), null);
  assertEquals(bootAuthDecision(now + 60_001, storage, true), null);
  assertEquals(bootAuthDecision(now + 60_001, storage, false), {
    view: "ready",
    me,
    cached: true,
    stale: true,
  });
  assertEquals(bootAuthDecision(now + 1_000, storage, false), { view: "ready", me, cached: true });
  // A sign-out or a login answer forgets the principal: no optimistic mount.
  rememberAuthStatus(status(me), now, storage);
  forgetAuthStatus(storage);
  assertEquals(bootAuthDecision(now + 1, storage), null);
});

test("an unreachable Cowboy opens the last principal even past its deadline", () => {
  const storage = memoryStorage();
  const now = 3_000_000;
  const me = { account: "draven", user_id: "user-a", role: "owner" as const };
  rememberAuthStatus(status(me, { idle_timeout_ms: 60_000 } as AuthStatus["session"]), now, storage);
  // Fresh: the ordinary cached decision, no stale mark.
  assertEquals(cachedAuthDecision({ view: "retry" }, now + 1_000, storage), { view: "ready", me, cached: true });
  assertEquals(readStaleAuthStatus(now + 1_000, storage)?.stale, false);
  // Past the idle window: still mounts for a retry probe, marked stale.
  assertEquals(readCachedAuthStatus(now + 60_001, storage), null);
  assertEquals(readStaleAuthStatus(now + 60_001, storage)?.stale, true);
  assertEquals(cachedAuthDecision({ view: "retry" }, now + 60_001, storage), {
    view: "ready",
    me,
    cached: true,
    stale: true,
  });
  // A real answer always wins over the stale principal.
  assertEquals(cachedAuthDecision({ view: "login" }, now + 60_001, storage), null);
  assertEquals(cachedAuthDecision({ view: "activating" }, now + 60_001, storage), null);
  // Sign-out forgets it: nothing opens offline after an explicit sign-out.
  forgetAuthStatus(storage);
  assertEquals(readStaleAuthStatus(now + 60_001, storage), null);
  assertEquals(cachedAuthDecision({ view: "retry" }, now + 60_001, storage), null);
  assertEquals(bootAuthDecision(now + 60_001, storage, false), null);
});
