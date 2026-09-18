import { assertEquals } from "jsr:@std/assert";
import type { AuthStatus } from "./authApi.ts";
import {
  authStatusCacheExpiry,
  cachedAuthDecision,
  forgetAuthStatus,
  readCachedAuthStatus,
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

Deno.test("a cached principal mounts only for retry probes and only before its deadline", () => {
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

Deno.test("expiry follows the earliest known deadline and auth-off never expires", () => {
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

Deno.test("a status without a user id clears the cache instead of storing a label", () => {
  const storage = memoryStorage();
  rememberAuthStatus(status({ account: "a", user_id: "u", role: "owner" }), 1, storage);
  rememberAuthStatus(status({ account: "legacy", role: "owner" }), 2, storage);
  assertEquals(readCachedAuthStatus(3, storage), null);
  storage.setItem("cowboy:auth-status-cache", "{not json");
  assertEquals(readCachedAuthStatus(3, storage), null);
});
