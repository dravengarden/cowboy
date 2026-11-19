import { productSessionSignal } from "./productSessionEnd";

/** Account controls of named-only Providers (Antigravity):
 * `accounts.list().account_controls` (docs/antigravity-real-accounts-design.md,
 * "Account controls", "Kill switch and logout", "UX"). Pure presentation and
 * the two owner writes; the Controller enforces every rule again. */
export type AccountControlState =
  | "ok" | "verification_required" | "location_unsupported" | "rate_limited"
  | "suspended" | "blocked" | "unknown_refusal" | "needs_attention";

export interface AccountControlFence { machine_id: string; action: string; state: string; detail?: string | null }
export interface UncertainEffect { session_id: string; method: string; sequence: number }
export interface AccountLogoutTarget {
  machine_id: string; state: string;
  credential_cleared?: boolean | null; history_retained?: boolean | null; detail?: string | null;
}
/** The plan the Provider last reported during one of the account's sessions
 * (informational; Cowboy never queries the vendor for it). */
export interface AccountPlan { id: string; name: string; observed_at_ms?: number }
export interface AccountLogout {
  operation_id: string; revision: number; requested_at_ms: number;
  state: "pending" | "signed_out" | "partially_signed_out" | "failed" | "unconfirmed" | "cancelled" | "not_signed_in" | string;
  targets: AccountLogoutTarget[];
}
export interface AccountControl {
  account_id: string;
  provider_id: string;
  admitted: boolean;
  state: AccountControlState | string;
  reason?: string | null;
  until_ms?: number | null;
  revision: number;
  /** The session whose live worker holds the account's single runtime. */
  in_use_by?: string | null;
  fences: AccountControlFence[];
  stop_confirmed: boolean;
  uncertain_effects: UncertainEffect[];
  logout?: AccountLogout | null;
  plan?: AccountPlan | null;
}
export interface AccountControls { loaded: boolean; accounts: AccountControl[] }

type Record_ = Record<string, unknown>;
function record(value: unknown): value is Record_ {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function text(value: unknown): value is string { return typeof value === "string" && value.length > 0; }
function optional(value: unknown, check: (value: unknown) => boolean): boolean {
  return value === undefined || value === null || check(value);
}
const number = (value: unknown) => typeof value === "number" && Number.isFinite(value);

function validControl(value: unknown): value is AccountControl {
  return record(value) && text(value.account_id) && text(value.provider_id) && typeof value.admitted === "boolean" &&
    text(value.state) && optional(value.reason, (reason) => typeof reason === "string") &&
    optional(value.until_ms, number) && number(value.revision) && typeof value.stop_confirmed === "boolean" &&
    optional(value.in_use_by, text) &&
    Array.isArray(value.fences) && value.fences.every((fence) => record(fence) && text(fence.machine_id) && text(fence.action) && text(fence.state)) &&
    Array.isArray(value.uncertain_effects) && value.uncertain_effects.every((effect) =>
      record(effect) && text(effect.session_id) && text(effect.method) && number(effect.sequence)) &&
    optional(value.logout, (logout) => record(logout) && text(logout.operation_id) && text(logout.state) &&
      Array.isArray(logout.targets) && logout.targets.every((target) => record(target) && text(target.machine_id) && text(target.state)));
}

/** The account's reported plan as card text, or nothing when the Controller
 * reported none or a value this client cannot show safely. A bad plan never
 * invalidates the account's controls. */
export function accountPlanLabel(control: Pick<AccountControl, "plan">): string | undefined {
  const plan: unknown = control.plan;
  if (!record(plan) || !text(plan.id) || !/^[a-z0-9][a-z0-9_-]{0,63}$/.test(plan.id) || !text(plan.name)) return undefined;
  const name = plan.name;
  if (name.trim() !== name || [...name].length > 80 || /[\p{C}\p{Zl}\p{Zp}]/u.test(name)) return undefined;
  return name;
}

/** The plans the account's sessions currently report in their usage
 * readings, as one comparable key. A change means the Controller's account
 * plan may have changed, so the account controls are worth reloading. */
export function sessionPlanKey(
  sessions: readonly { provider_account_id?: string; usage?: { raw: Record<string, unknown> | null } }[],
  account: string,
): string {
  const plans = new Set<string>();
  for (const session of sessions) {
    if (session.provider_account_id !== account) continue;
    const meta = session.usage?.raw?._meta;
    const plan = record(meta) && record(meta.cowboy) ? meta.cowboy.account_plan : undefined;
    if (record(plan)) plans.add(JSON.stringify([plan.id, plan.name]));
  }
  return [...plans].sort().join("\n");
}

export function validAccountControls(value: unknown): value is AccountControls {
  return record(value) && typeof value.loaded === "boolean" && Array.isArray(value.accounts) && value.accounts.every(validControl);
}

/** The snapshot fields this module reads. */
export interface ControlledAccounts {
  named_only_providers?: string[];
  account_controls?: AccountControls;
}

/** A Provider whose sessions need a named account (no default account). */
export function namedOnlyProvider(data: ControlledAccounts | undefined, provider: string): boolean {
  return data?.named_only_providers?.includes(provider) === true;
}

export function accountControl(data: ControlledAccounts | undefined, account: string | undefined | null): AccountControl | undefined {
  return account ? data?.account_controls?.accounts.find((control) => control.account_id === account) : undefined;
}

/** The control a choice may rely on: none until the Controller's controls
 * snapshot is loaded, so the picker never enables an account from a
 * partial or failed load. */
export function loadedAccountControl(data: ControlledAccounts | undefined, account: string | undefined | null): AccountControl | undefined {
  return data?.account_controls?.loaded === true ? accountControl(data, account) : undefined;
}

/** `Nov 18, 14:05` in the viewer's locale. */
export function controlTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

const STATE_LABELS: Record<string, string> = {
  verification_required: "Verification required",
  location_unsupported: "Location unsupported",
  rate_limited: "Rate limited",
  suspended: "Suspended",
  blocked: "Blocked",
  unknown_refusal: "Unknown refusal",
  needs_attention: "Needs attention",
};

/** The owner-facing text of a control reason code. */
export function controlReasonText(reason: string | null | undefined): string | undefined {
  if (!reason) return undefined;
  if (reason.startsWith("policy_violation:")) return `A built-in tool ran (${reason.slice("policy_violation:".length)})`;
  const known: Record<string, string> = {
    owner_block: "Blocked by the owner",
    owner_unblock: "Unblocked by the owner",
    owner_clear: "Cleared by the owner",
    owner_send_after_rate_limit: "Resumed by an owner send",
    unknown_refusal: "A turn ended without a classified backend outcome",
  };
  return known[reason] ?? reason.replaceAll("_", " ");
}

export type BadgeTone = "success" | "warning" | "error" | "default";
export interface StateBadge { label: string; tone: BadgeTone; detail?: string | undefined }

/** The card's state badge. `signIn` is the account's sign-in observation:
 * `signed_in`, `not_signed_in`, `pending` or unknown (undefined). Only an
 * affirmative observation shows "Signed in"; unknown or loading is neutral. */
export function accountStateBadge(control: AccountControl, signIn: "signed_in" | "not_signed_in" | "pending" | undefined, now = Date.now()): StateBadge {
  const detail = controlReasonText(control.reason);
  if (control.state === "ok") {
    if (signIn === "signed_in") return { label: "Signed in", tone: "success" };
    if (signIn === "not_signed_in") return { label: "Not signed in", tone: "warning" };
    if (signIn === "pending") return { label: "Sign-in not finished", tone: "warning" };
    return { label: "Sign-in unknown", tone: "default" };
  }
  if (control.state === "rate_limited") {
    const until = control.until_ms ?? undefined;
    return until !== undefined && until > now
      ? { label: `Rate limited until ${controlTime(until)}`, tone: "warning", detail }
      : { label: "Rate limit ended · send again to resume", tone: "warning", detail };
  }
  const label = STATE_LABELS[control.state] ?? "Unknown refusal";
  const tone: BadgeTone = control.state === "blocked" || control.state === "suspended" ? "error" : "warning";
  return { label, tone, detail };
}

export interface ControlActions {
  clear: boolean; block: boolean; unblock: boolean; logout: boolean; stopUnits: boolean;
}

/** Which owner actions the card offers now (the Controller rechecks each). */
export function controlActions(control: AccountControl): ControlActions {
  const blocked = control.state === "blocked";
  const logout = control.logout;
  const loggedOutThisBlock = logout !== undefined && logout !== null && logout.revision === control.revision &&
    ["pending", "signed_out", "not_signed_in"].includes(logout.state);
  return {
    clear: control.state !== "ok" && !blocked,
    block: !blocked,
    unblock: blocked,
    logout: blocked && control.stop_confirmed && !loggedOutThisBlock,
    stopUnits: control.fences.some((fence) => fence.state !== "confirmed"),
  };
}

/** The owner-facing line of the kill switch's logout. */
export function logoutStatus(logout: AccountLogout | null | undefined): string | undefined {
  if (!logout) return undefined;
  const offline = logout.targets.filter((target) => target.state === "pending").map((target) => target.machine_id);
  switch (logout.state) {
    case "pending": return `Signing out · waiting for ${offline.join(", ") || "the Machine"}`;
    case "signed_out": return "Signed out · history kept";
    case "partially_signed_out": return "Signed out on some Machines · history kept";
    case "not_signed_in": return "Nothing to sign out: no Machine was signed in";
    case "failed": return "Sign-out failed on a Machine";
    case "unconfirmed": return "Sign-out outcome is unknown on a Machine";
    case "cancelled": return "Sign-out cancelled: the account was unblocked first";
    default: return `Sign-out ${logout.state.replaceAll("_", " ")}`;
  }
}

/** Why a new session cannot use this named-only account, if it cannot:
 * only admitted, signed-in, `ok` accounts no other session runs are
 * choosable. */
export function pickerUnavailableReason(control: AccountControl | undefined, loginReason: string | undefined, now = Date.now()): string | undefined {
  if (!control) return "Account state not loaded";
  if (!control.admitted) return "Not admitted for sessions";
  if (control.state !== "ok") {
    const badge = accountStateBadge(control, undefined, now);
    return badge.detail && control.state !== "blocked" ? `${badge.label} · ${badge.detail}` : badge.label;
  }
  // One runtime per account: another session's live worker holds it.
  if (control.in_use_by) return `In use by another session (${control.in_use_by}); hibernate it first`;
  return loginReason;
}

export interface ParkedReason {
  text: string;
  /** Another session's live runtime holds the account. */
  waitingFor?: string | undefined;
  /** An uncertain target effect needs the owner's acknowledgement. */
  uncertain?: UncertainEffect | undefined;
}

const WAIT_PREFIX = "Waiting for resources: ";

/** Why a session of a governed account cannot send now. `wait` is the
 * session's server-reported resource wait (the parked queue's reason). */
export function sessionParkedReason(control: AccountControl | undefined, session: string, wait: string | undefined, now = Date.now()): ParkedReason | undefined {
  const uncertain = control?.uncertain_effects.find((effect) => effect.session_id === session);
  if (uncertain) {
    return { text: `The outcome of a target ${uncertain.method} is unknown. Check the target, then acknowledge it to resume.`, uncertain };
  }
  if (control && !control.admitted) return { text: "This account is not admitted for sessions." };
  if (control && control.state !== "ok") {
    const badge = accountStateBadge(control, undefined, now);
    return { text: badge.detail ? `${badge.label} · ${badge.detail}` : badge.label };
  }
  if (control?.in_use_by && control.in_use_by !== session) {
    return { text: "In use by another session; hibernate it first.", waitingFor: control.in_use_by };
  }
  if (wait) return { text: wait.startsWith(WAIT_PREFIX) ? wait.slice(WAIT_PREFIX.length) : wait };
  return undefined;
}

/** The text of a refused control action. */
export function controlErrorText(code: string | undefined): string {
  switch (code) {
    case "confirmation_required": return "Confirm the action first.";
    case "account_not_blocked": return "Block the account first.";
    case "account_stop_unconfirmed": return "Wait until every unit of the account is confirmed stopped.";
    case "account_blocked_use_unblock": return "Use Unblock for a blocked account.";
    case "expected_revision_required":
    case "revision_conflict": return "The account changed. Review its refreshed state and try again.";
    case "permission_denied": return "Only the Service owner can change admission.";
    case "antigravity_account_in_use": return "In use by another session; hibernate it first.";
    case "no_uncertain_effect": return "Nothing is waiting for acknowledgement.";
    case "uncertain_effect_changed": return "Another uncertain effect arrived. Review it before acknowledging.";
    case "outcome_unknown": return "The outcome is not confirmed. Check the account before retrying.";
    default: return `Action refused (${code ?? "unavailable"}).`;
  }
}

export type ControlAction = "admit" | "unadmit" | "clear" | "block" | "unblock" | "stop_units" | "logout";

/** The request body of one control action. */
export function controlRequest(action: ControlAction, control: AccountControl): { action: ControlAction; expected_revision?: number; confirm?: boolean } {
  if (action === "clear" || action === "unblock") return { action, expected_revision: control.revision };
  if (action === "block" || action === "logout") return { action, confirm: true };
  return { action };
}

async function post(path: string, body: unknown): Promise<void> {
  const response = await fetch(path, {
    method: "POST", headers: { "Content-Type": "application/json" }, signal: productSessionSignal(), body: JSON.stringify(body),
  });
  if (response.ok) return;
  const value = await response.json().catch(() => null) as { error?: { code?: string } } | null;
  throw new Error(controlErrorText(value?.error?.code));
}

export function postAccountControl(account: string, action: ControlAction, control: AccountControl): Promise<void> {
  return post(`/api/provider-accounts/${encodeURIComponent(account)}/controls`, controlRequest(action, control));
}

export function acknowledgeUncertainEffect(session: string, sequence: number): Promise<void> {
  return post(`/api/sessions/${encodeURIComponent(session)}/uncertain-effect/acknowledge`, { sequence });
}

/** The sign-in state of an account from its cached usage observation
 * (`/api/provider-accounts/usage`), never a Provider request. */
export function signInObservation(observation: { reason?: string | undefined; available?: boolean | undefined; signed_in?: boolean | undefined } | undefined): "signed_in" | "not_signed_in" | "pending" | undefined {
  if (!observation) return undefined;
  // The server's verified native login, reported also when the Provider has
  // no usage collector (Antigravity): sign-in is independent of quota.
  if (observation.signed_in === true) return "signed_in";
  if (observation.reason === "native_login_required" || observation.reason === "native_login_outdated") return "not_signed_in";
  if (observation.reason === "native_login_pending") return "pending";
  // Only the server's resolved native login is evidence of a sign-in; any
  // other refusal (disabled, unavailable, an unknown code) proves nothing.
  return observation.available === true ? "signed_in" : undefined;
}

/** The uncertain effect an acknowledgement sheet was opened for. */
export interface AcknowledgementTarget { session: string; method: string; sequence: number }

/** What the open sheet may acknowledge: exactly the effect the owner was
 * shown when it opened, and only while it is still the session's current
 * effect. `undefined` once the session or the effect changed. */
export function acknowledgementTarget(
  captured: AcknowledgementTarget | undefined, session: string, current: UncertainEffect | undefined,
): AcknowledgementTarget | undefined {
  if (!captured || !current) return undefined;
  return captured.session === session && captured.method === current.method && captured.sequence === current.sequence
    ? captured : undefined;
}
