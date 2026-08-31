import { assertEquals } from "jsr:@std/assert";
import type { ProductMe } from "./authApi.ts";
import { sessionAlertState, sessionCountdownLabel } from "./sessionSchedule.ts";

const base: ProductMe = {
  account: "draven",
  role: "owner",
  passkey_count: 1,
  passkey_reauth_enabled: true,
  primary_reauth_due_at_ms: 100_000,
  primary_reauth_warn_at_ms: 80_000,
  passkey_reauth_due_at_ms: 50_000,
  passkey_reauth_warn_at_ms: 40_000,
  session_idle_due_at_ms: 60_000,
};

Deno.test("session schedule warns for the earliest proof and makes primary login authoritative when due", () => {
  assertEquals(sessionAlertState(base, 45_000), {
    kind: "passkey",
    phase: "warning",
    dueAtMs: 50_000,
  });
  assertEquals(sessionAlertState(base, 50_000)?.kind, "passkey");
  assertEquals(sessionAlertState(base, 100_000), {
    kind: "primary",
    phase: "required",
    dueAtMs: 100_000,
  });
});

Deno.test("session schedule falls back to full login when an idle session has no Passkey", () => {
  assertEquals(
    sessionAlertState({
      ...base,
      passkey_count: 0,
      passkey_reauth_enabled: false,
      passkey_reauth_due_at_ms: null,
    }, 60_000)?.kind,
    "primary",
  );
});

Deno.test("session schedule formats a compact, stable countdown", () => {
  assertEquals(sessionCountdownLabel(3_661_000), "1h 2m");
  assertEquals(sessionCountdownLabel(65_000), "1m 05s");
  assertEquals(sessionCountdownLabel(-1), "0s");
});
