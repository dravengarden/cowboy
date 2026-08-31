import type { ProductMe } from "./authApi";

export type SessionVerificationKind = "passkey" | "primary";

export interface SessionAlertState {
  kind: SessionVerificationKind;
  phase: "warning" | "required";
  dueAtMs: number;
}

function passkeyEligible(me: ProductMe): boolean {
  return me.passkey_reauth_enabled === true && (me.passkey_count ?? 0) > 0;
}

export function sessionAlertState(
  me: ProductMe,
  nowMs: number,
): SessionAlertState | null {
  const primaryDue = me.primary_reauth_due_at_ms ?? null;
  const passkeyDue = me.passkey_reauth_due_at_ms ?? null;
  const idleDue = me.session_idle_due_at_ms ?? null;
  if (
    me.session_reauth_kind === "primary" ||
    primaryDue != null && nowMs >= primaryDue
  ) {
    return { kind: "primary", phase: "required", dueAtMs: primaryDue ?? nowMs };
  }
  if (
    me.session_reauth_kind === "passkey" ||
    passkeyDue != null && nowMs >= passkeyDue ||
    idleDue != null && nowMs >= idleDue
  ) {
    return {
      kind: passkeyEligible(me) ? "passkey" : "primary",
      phase: "required",
      dueAtMs: Math.min(
        passkeyDue ?? Number.MAX_SAFE_INTEGER,
        idleDue ?? Number.MAX_SAFE_INTEGER,
      ),
    };
  }

  const warnings: SessionAlertState[] = [];
  if (
    primaryDue != null &&
    me.primary_reauth_warn_at_ms != null &&
    nowMs >= me.primary_reauth_warn_at_ms
  ) {
    warnings.push({ kind: "primary", phase: "warning", dueAtMs: primaryDue });
  }
  if (
    passkeyEligible(me) &&
    passkeyDue != null &&
    me.passkey_reauth_warn_at_ms != null &&
    nowMs >= me.passkey_reauth_warn_at_ms
  ) {
    warnings.push({ kind: "passkey", phase: "warning", dueAtMs: passkeyDue });
  }
  return warnings.sort((left, right) => left.dueAtMs - right.dueAtMs)[0] ??
    null;
}

export function sessionCountdownLabel(remainingMs: number): string {
  const seconds = Math.max(0, Math.ceil(remainingMs / 1_000));
  if (seconds >= 3_600) {
    const totalMinutes = Math.ceil(seconds / 60);
    const hours = Math.floor(totalMinutes / 60);
    const minutes = totalMinutes % 60;
    return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
  }
  if (seconds >= 60) {
    const minutes = Math.floor(seconds / 60);
    const tail = seconds % 60;
    return `${minutes}m ${tail.toString().padStart(2, "0")}s`;
  }
  return `${seconds}s`;
}

export function nextSessionClockDelay(
  me: ProductMe,
  nowMs: number,
  alert: SessionAlertState | null,
): number {
  if (alert) {
    return alert.dueAtMs - nowMs <= 60 * 60 * 1_000 ? 1_000 : 60_000;
  }
  const boundaries = [
    me.passkey_reauth_warn_at_ms,
    me.primary_reauth_warn_at_ms,
    me.session_idle_due_at_ms,
  ].filter((value): value is number =>
    typeof value === "number" && value > nowMs
  );
  if (boundaries.length === 0) return 60_000;
  return Math.max(250, Math.min(60_000, Math.min(...boundaries) - nowMs));
}
