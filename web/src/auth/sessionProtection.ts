import type {
  ProductMe,
  ProductPasskeyServerPolicy,
  ProductSessionServerPolicy,
} from "./authApi.ts";

const MINUTE_MS = 60 * 1_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

export interface SessionProtectionItem {
  label: string;
  value: string;
}

function unit(value: number, singular: string, plural: string): string {
  return `${value} ${value === 1 ? singular : plural}`;
}

export function sessionPolicyDuration(durationMs: number): string {
  if (durationMs >= DAY_MS && durationMs % DAY_MS === 0) {
    return unit(durationMs / DAY_MS, "day", "days");
  }
  if (durationMs >= HOUR_MS && durationMs % HOUR_MS === 0) {
    return unit(durationMs / HOUR_MS, "hour", "hours");
  }
  if (durationMs >= MINUTE_MS && durationMs % MINUTE_MS === 0) {
    return unit(durationMs / MINUTE_MS, "minute", "minutes");
  }
  const seconds = Math.max(0, Math.round(durationMs / 1_000));
  return unit(seconds, "second", "seconds");
}

export function sessionDeadlineLabel(
  dueAtMs: number | null | undefined,
  serverNowMs: number,
): string {
  if (dueAtMs == null) return "Not scheduled";
  const remainingMs = dueAtMs - serverNowMs;
  if (remainingMs <= 0) return "Required now";

  const totalMinutes = Math.ceil(remainingMs / MINUTE_MS);
  const days = Math.floor(totalMinutes / (24 * 60));
  const hours = Math.floor((totalMinutes % (24 * 60)) / 60);
  const minutes = totalMinutes % 60;
  if (days > 0) return `Due in ${days}d${hours > 0 ? ` ${hours}h` : ""}`;
  if (hours > 0) return `Due in ${hours}h${minutes > 0 ? ` ${minutes}m` : ""}`;
  return `Due in ${Math.max(1, minutes)}m`;
}

export function currentSessionProtectionItems(
  me: ProductMe,
  passkeys: ProductPasskeyServerPolicy | undefined,
  session: ProductSessionServerPolicy | undefined,
  serverNowMs = me.session_server_now_ms ?? Date.now(),
): SessionProtectionItem[] {
  const passkeyCount = me.passkey_count ?? 0;
  let passkeyValue = "Policy unavailable";
  if (passkeys?.enabled === false) {
    passkeyValue = "Disabled by service";
  } else if (passkeyCount === 0) {
    passkeyValue = "Set up a Passkey";
  } else if (passkeys?.session_refresh_enabled === false) {
    passkeyValue = "Session checks disabled";
  } else if (me.passkey_reauth_enabled !== true) {
    passkeyValue = "Off for this account";
  } else if (me.passkey_reauth_due_at_ms == null) {
    passkeyValue = "Verify this browser";
  } else {
    passkeyValue = sessionDeadlineLabel(
      me.passkey_reauth_due_at_ms,
      serverNowMs,
    );
  }

  return [
    { label: "This browser", value: `Signed in as ${me.account}` },
    {
      label: "Idle sign-out",
      value: session?.activity_sliding_enabled === false
        ? "Activity extension off"
        : sessionDeadlineLabel(me.session_idle_due_at_ms, serverNowMs),
    },
    { label: "Passkey check", value: passkeyValue },
    {
      label: "Full sign-in",
      value: sessionDeadlineLabel(me.primary_reauth_due_at_ms, serverNowMs),
    },
  ];
}

export function configuredSessionProtectionItems(
  passkeys: ProductPasskeyServerPolicy,
  session: ProductSessionServerPolicy,
): SessionProtectionItem[] {
  return [
    {
      label: "Activity extends idle timer",
      value: session.activity_sliding_enabled ? "On" : "Off",
    },
    {
      label: "Idle timeout",
      value: sessionPolicyDuration(session.idle_timeout_ms),
    },
    { label: "Passkeys", value: passkeys.enabled ? "Enabled" : "Disabled" },
    {
      label: "Prompt after sign-in",
      value: passkeys.prompt_after_login ? "On" : "Off",
    },
    {
      label: "Passkey session extension",
      value: passkeys.session_refresh_enabled ? "Allowed" : "Disabled",
    },
    {
      label: "Maximum Passkey interval",
      value: sessionPolicyDuration(session.passkey_max_age_ms),
    },
    {
      label: "Passkey reminder",
      value: `${sessionPolicyDuration(session.passkey_warning_ms)} before`,
    },
    {
      label: "Full sign-in limit",
      value: sessionPolicyDuration(session.primary_max_age_ms),
    },
    {
      label: "Full sign-in reminder",
      value: `${sessionPolicyDuration(session.primary_warning_ms)} before`,
    },
  ];
}
