import { eventsFrom, type ProviderLoginEvent } from "./providerAuthenticationOwner";

export interface LoginScope { owner: string; dataset: string; account: string; provider: string }
export interface LoginOptions {
  account_id: string; provider_id: string; machine_id: string; available: boolean; reason: string | null;
  release: { provider_version: string; generation_digest: string } | null;
  methods: { id: string; label?: string; available: boolean; reason: string | null }[];
  native_profile?: { generation: number } | null;
}
type Pending = LoginScope & { machine: string; cancelling: boolean; codeState?: "submitting" | "accepted" | "unconfirmed" } & (
  | { operation: string; version: string; digest: string; method: string; request?: never }
  | { request: string; operation?: never; version?: never; digest?: never; method?: never }
);
export interface LoginObservation { status: string; request_id?: string; cancel_requested?: boolean }
export interface LoginView {
  pending?: Pending | undefined; observation?: LoginObservation | undefined; events: ProviderLoginEvent[]; busy: boolean; error: string;
  referenceMismatch?: boolean;
  checking?: boolean;
  submissionError?: string | undefined;
}
class CodeRejected extends Error {
  constructor(status: number) {
    super(status === 401 || status === 403 || status === 428
      ? "The code was not sent. Refresh your Cowboy sign-in, then reopen this account’s existing sign-in and enter the code again."
      : `The code was not sent (HTTP ${status}). Check status, then enter the code again if this sign-in is still active.`);
  }
}
export function loginCompleted(view: LoginView): boolean {
  return view.observation?.status === "completed" && !view.observation.cancel_requested && !view.pending?.cancelling;
}
export function loginFailure(view: LoginView): string | undefined {
  const failure = view.events.findLast((event) =>
    (event.event === "login_state" && event.state === "error") ||
    (event.event === "command_result" && !event.accepted));
  return failure ? ("detail" in failure && failure.detail || "Native sign-in failed. Start again when the Machine is ready.") : undefined;
}
export function canReplaceLogin(view: LoginView): boolean {
  return Boolean(view.pending && (view.observation?.cancel_requested ||
    ["inactive", "not_started"].includes(view.observation?.status ?? "") ||
    (view.observation?.status === "active" && (loginFailure(view) !== undefined || (!view.pending.codeState && view.events.some((event) =>
      event.event === "login_challenge" && event.expires_at_ms <= Date.now()))))));
}
function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
const text = (value: unknown): value is string => typeof value === "string" && value.length > 0 && value.length <= 16384;
const id = (value: unknown): value is string => text(value) && /^[A-Za-z0-9_-]+$/.test(value);
export function loginOptions(value: unknown, scope: LoginScope, machine: string): LoginOptions {
  if (!object(value) || value.account_id !== scope.account || value.provider_id !== scope.provider || value.machine_id !== machine ||
    typeof value.available !== "boolean" || !(value.reason === null || text(value.reason)) ||
    !(value.release === null || (object(value.release) && text(value.release.provider_version) && text(value.release.generation_digest))) ||
    !(value.native_profile == null || (object(value.native_profile) && Number.isSafeInteger(value.native_profile.generation) && Number(value.native_profile.generation) > 0)) ||
    !Array.isArray(value.methods) || !value.methods.every((method) => object(method) && id(method.id) && (method.label === undefined || text(method.label)) && typeof method.available === "boolean" && (method.reason === null || text(method.reason)))) {
    throw new Error("Invalid subscription login options.");
  }
  return value as unknown as LoginOptions;
}
export function loginLink(value: string): string | undefined {
  try { const url = new URL(value); return url.protocol === "https:" && !url.username && !url.password ? url.href : undefined; }
  catch { return undefined; }
}
function restore(value: unknown, scope: LoginScope): Pending {
  if (!object(value) || Object.keys(value).some((key) => !["owner", "dataset", "account", "provider", "operation", "machine", "version", "digest", "method", "cancelling", "request", "codeState"].includes(key)) ||
    value.owner !== scope.owner || value.dataset !== scope.dataset || value.account !== scope.account || value.provider !== scope.provider ||
    !id(value.machine) || typeof value.cancelling !== "boolean" ||
    (value.codeState !== undefined && (typeof value.codeState !== "string" || !["submitting", "accepted", "unconfirmed"].includes(value.codeState))) ||
    (value.request !== undefined
      ? !id(value.request) || ["operation", "version", "digest", "method"].some((key) => key in value)
      : !id(value.operation) || !text(value.version) || !text(value.digest) || !id(value.method))) {
    throw new Error("Saved sign-in belongs to another account or dataset, or is invalid.");
  }
  return value as unknown as Pending;
}
function sameAttempt(left: Pending, right: Pending): boolean {
  return (["machine", "operation", "request", "version", "digest", "method"] as const).every((field) => left[field] === right[field]);
}

/** One persisted operation per account. Only metadata is stored, never codes or links.
 * No mutation on mount/unmount, automatic login retry, or vendor polling. */
export function subscriptionLogin(scope: LoginScope, ports: {
  fetch: (path: string, init?: RequestInit) => Promise<Response>;
  storage: Pick<Storage, "getItem" | "setItem" | "removeItem">;
  current: () => boolean;
  changed: (view: LoginView) => void;
  operation: () => string;
}) {
  const key = `cowboy:subscription-login:${scope.owner}:${scope.account}`;
  let state: LoginView = { events: [], busy: false, error: "" };
  let ready = false;
  const publish = (patch: Partial<LoginView>) => {
    state = { ...state, ...patch };
    if (ports.current()) ports.changed(state);
  };
  const saved = () => {
    const raw = ports.storage.getItem(key);
    return raw ? restore(JSON.parse(raw), scope) : undefined;
  };
  const save = (pending: Pending) => {
    const previous = saved();
    if (previous && !sameAttempt(previous, pending)) throw new Error("Another sign-in is saved in this tab. Reopen sign-in to continue it.");
    const codeState = pending.codeState ?? previous?.codeState;
    pending = { ...pending, cancelling: pending.cancelling || previous?.cancelling === true,
      ...(codeState ? { codeState } : {}) };
    ports.storage.setItem(key, JSON.stringify(pending));
    publish({ pending });
    return pending;
  };
  const forget = (pending: Pending) => {
    const previous = saved();
    if (previous && sameAttempt(previous, pending)) ports.storage.removeItem(key);
  };
  async function request(path: string, init?: RequestInit, reply: "json" | "accepted" = "json"): Promise<unknown> {
    if (!ports.current()) throw new Error("Account changed. Reopen subscriptions.");
    const response = await ports.fetch(path, { cache: "no-store", ...init });
    if (!ports.current()) throw new Error("Account changed. Reopen subscriptions.");
    // Code submission returns an empty 202. It only acknowledges dispatch;
    // the original operation receipt still decides whether sign-in completed.
    if (reply === "accepted" && response.status === 202) return;
    // The auth/code endpoint and product auth reject 4xx before enqueueing.
    // A known rejection can be corrected; transport/5xx failures remain unknown.
    if (reply === "accepted" && response.status >= 400 && response.status < 500) throw new CodeRejected(response.status);
    const value: unknown = await response.json().catch(() => undefined);
    if (!ports.current()) throw new Error("Account changed. Reopen subscriptions.");
    if (!response.ok) {
      const code = object(value) && object(value.error) && id(value.error.code) ? ` (${value.error.code})` : "";
      throw new Error(`Sign-in is not confirmed${code}. Check status or retry the same operation.`);
    }
    if (reply === "accepted" || value === undefined) {
      throw new Error("Sign-in response could not be verified. Check status before trying again.");
    }
    return value;
  }
  function observe(value: unknown, pending: Pending): void {
    if (!object(value) || !text(value.status) ||
      (value.cancel_requested !== undefined && typeof value.cancel_requested !== "boolean") ||
      (!["unknown", "cancellation_requested"].includes(value.status) &&
        (value.account_id !== pending.account || value.provider_id !== pending.provider || value.machine_id !== pending.machine || !id(value.request_id))) ||
      (value.request_id !== undefined && !id(value.request_id)) ||
      (value.account_id !== undefined && value.account_id !== pending.account) ||
      (value.provider_id !== undefined && value.provider_id !== pending.provider) ||
      (value.machine_id !== undefined && value.machine_id !== pending.machine) ||
      (pending.request !== undefined && value.request_id !== pending.request) ||
      (state.observation?.request_id && value.request_id && state.observation.request_id !== value.request_id)) {
      throw new Error("Sign-in response does not match this operation.");
    }
    // Keep the same live challenge mounted during receipt polling. Unmounting
    // it every two seconds discards a code while the user is entering it.
    // Terminal/cancellation transitions and all read failures still clear it.
    const sameActive = value.status === "active" && !value.cancel_requested && !pending.cancelling &&
      value.request_id === state.observation?.request_id;
    publish({ observation: value as unknown as LoginObservation, ...(sameActive ? {} : { events: [] }) });
  }
  const receiptPath = (pending: Pending) => pending.request
    ? `/api/provider-accounts/${scope.account}/logins/${pending.request}`
    : `/api/provider-accounts/login-operations/${pending.operation}`;
  async function check(pending: Pending) {
    const value = await request(receiptPath(pending));
    observe(value, pending);
    await readEvents(pending);
  }
  async function readEvents(pending: Pending) {
    const observation = state.observation;
    if (observation?.status === "active" && observation.request_id && !observation.cancel_requested && !pending.cancelling) {
      const events = await request(`/api/providers/${scope.provider}/auth/${observation.request_id}`);
      publish({ events: eventsFrom(events, scope.provider, observation.request_id) });
    }
  }
  async function run(action: () => Promise<void>, checking = false) {
    if (!ready || state.busy || !ports.current()) return;
    publish({ busy: true, checking, error: "" });
    try { await action(); } catch (error) {
      publish({ error: error instanceof Error ? error.message : "Sign-in is not confirmed. Check status.", events: [], observation: undefined });
    } finally { publish({ busy: false, checking: false }); }
  }
  async function send(pending: Pending) {
    if (pending.codeState) throw new Error("An authorization code was already submitted. Check the original sign-in.");
    if (pending.request) throw new Error("Continue this existing sign-in with Check status. It cannot be restarted here.");
    if (pending.cancelling) throw new Error("Cancellation is pending. Check status to reconcile this attempt.");
    const value = await request(`/api/provider-accounts/${scope.account}/login`, {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ operation: pending.operation, machine_id: pending.machine, provider_version: pending.version,
        generation_digest: pending.digest, method: pending.method }),
    });
    observe(value, pending);
    await check(pending);
  }
  return {
    get snapshot() { return state; },
    restore() {
      try {
        const saved = ports.storage.getItem(key);
        if (saved) {
          const pending = restore(JSON.parse(saved), scope);
          // A remount cannot know whether an in-flight write reached the Machine.
          publish({ pending: pending.codeState === "submitting" ? { ...pending, codeState: "unconfirmed" } : pending });
        }
        ready = true;
      } catch { publish({ error: "Saved sign-in could not be read. No new login will start." }); }
    },
    resume(reference: string) { return run(async () => {
      if (!id(reference)) throw new Error("Invalid sign-in reference.");
      const mismatch = () => {
        publish({ referenceMismatch: true });
        return new Error("Another sign-in is saved in this tab. Reconcile it in Subscriptions first.");
      };
      if (state.pending) {
        if (state.observation?.request_id && state.observation.request_id !== reference) throw mismatch();
        if (state.pending.request && state.pending.request !== reference) throw mismatch();
        const value = await request(receiptPath(state.pending));
        if (!object(value) || !id(value.request_id)) throw new Error("The original sign-in is not confirmed. Check status.");
        if (value.request_id !== reference) throw mismatch();
        observe(value, state.pending);
        publish({ referenceMismatch: false });
        await readEvents(state.pending);
        return;
      }
      const value = await request(`/api/provider-accounts/${scope.account}/logins/${reference}`);
      if (!object(value) || !id(value.machine_id) || value.request_id !== reference) throw new Error("Sign-in could not be found for this account.");
      const pending: Pending = { ...scope, request: reference, machine: value.machine_id, cancelling: value.cancel_requested === true };
      observe(value, pending);
      publish({ referenceMismatch: false });
      save(pending); // Only a verified reference is persisted; never a new dispatch selection.
      await readEvents(pending);
    }, true); },
    start(options: LoginOptions, method: string) { return run(async () => {
      if (state.pending) throw new Error("Reconcile the existing sign-in first.");
      options = loginOptions(options, scope, options.machine_id);
      if (!options.available || !options.release || !options.methods.some((entry) => entry.id === method && entry.available)) throw new Error("Sign-in is unavailable on this Machine.");
      const pending: Pending = { ...scope, operation: ports.operation(), machine: options.machine_id,
        version: options.release.provider_version, digest: options.release.generation_digest, method, cancelling: false };
      save(restore(pending, scope)); // Durable reservation before any POST.
      await send(pending);
    }); },
    retry() { return run(async () => { if (state.pending) await send(state.pending); }); },
    check() { return run(async () => { if (state.pending) await check(state.pending); }, true); },
    cancel() { return run(async () => {
      if (!state.pending) return;
      const pending = { ...state.pending, cancelling: true };
      save(pending);
      observe(await request(receiptPath(pending), { method: "DELETE" }), pending);
    }); },
    prepareNew() { return run(async () => {
      if (!state.pending || !canReplaceLogin(state)) throw new Error("Check or cancel the original sign-in first.");
      if (!state.observation?.cancel_requested && state.observation?.status !== "not_started") {
        const pending = { ...state.pending, cancelling: true };
        save(pending);
        observe(await request(receiptPath(pending), { method: "DELETE" }), pending);
        if (!state.observation?.cancel_requested) throw new Error("Cancellation is not acknowledged. Check the original sign-in.");
      }
      // An explicit new selection, not a replay of the old request. Do not
      // POST here: fresh readiness and a separate user click are still needed.
      // The Machine retains the old receipt/profile and all cleanup fences.
      forget(state.pending);
      publish({ pending: undefined, observation: undefined, events: [], error: "", submissionError: undefined });
    }); },
    submit(code: string) { return run(async () => {
      const pending = state.pending; const observation = state.observation;
      if (!pending || pending.cancelling || observation?.status !== "active" || observation.cancel_requested || !observation.request_id) throw new Error("Sign-in is not accepting a code.");
      if (pending.codeState) throw new Error("An authorization code was already submitted. Check the original sign-in.");
      code = code.trim();
      if (!code || new TextEncoder().encode(code).length > 4096 || Array.from(code).some((character) => character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127)) throw new Error("Enter one authorization code.");
      // Persist only the attempt marker before sending. Reloads and lost replies
      // recover this request; neither the code nor an automatic retry is stored.
      const previous = saved();
      if (!previous || !sameAttempt(previous, pending) || previous.cancelling) throw new Error("The saved sign-in changed. Reopen sign-in before continuing.");
      if (previous.codeState) throw new Error("An authorization code was already submitted. Check the original sign-in.");
      publish({ submissionError: undefined });
      save({ ...pending, codeState: "submitting" });
      let codeState: Pending["codeState"] = "accepted";
      let submissionError: string | undefined;
      try {
        await request(`/api/providers/${scope.provider}/auth/${observation.request_id}`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ code }) }, "accepted");
      } catch (error) {
        if (error instanceof CodeRejected) { codeState = undefined; submissionError = error.message; }
        else codeState = "unconfirmed";
      }
      if (!ports.current()) return;
      let submitted: Pending = { ...pending, ...(codeState ? { codeState } : {}) };
      const current = saved();
      if (current && sameAttempt(current, pending)) {
        if (submissionError && current.codeState === "submitting") {
          // Clear only our rejected submission, preserving concurrent cancellation.
          submitted = { ...current }; delete submitted.codeState;
          ports.storage.setItem(key, JSON.stringify(submitted));
          publish({ pending: submitted });
        } else submitted = save(submitted);
      }
      else publish({ pending: submitted }); // Do not recreate or replace a newer tab pointer.
      publish({ submissionError });
      // Even a lost/rejected delivery reply can race with completed native
      // authorization. Only the scoped receipt may settle that uncertainty.
      await check(submitted);
    }); },
    finish() {
      // Only forget the tab pointer after completion or a durable cancellation
      // acknowledgement. The server retains cancellation and all cleanup fences;
      // this does not assert that the native process exited or dispatch a retry.
      if (state.busy || !ports.current() || (!state.observation?.cancel_requested && !loginCompleted(state) && state.observation?.status !== "not_started")) return;
      try {
        const previous = saved();
        if (previous && state.pending && sameAttempt(previous, state.pending) && previous.cancelling && !state.observation?.cancel_requested) {
          publish({ pending: previous });
          return;
        }
        if (state.pending) forget(state.pending);
        publish({ pending: undefined, observation: undefined, events: [], error: "", submissionError: undefined });
      }
      catch { publish({ error: "Could not clear the saved sign-in. It remains available for reconciliation." }); }
    },
  };
}
