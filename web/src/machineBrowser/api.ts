/** Typed client for the Controller's Machine browser routes under
 * `/api/provider-accounts/{account}/browser*`, the shapes they answer with and
 * the people-facing text for refusal codes and lifecycle reasons. Codes stay in
 * the API for agents and diagnostics; unknown future codes fall back to a
 * neutral text. See docs/machine-browser.md, "Controller routes and authority"
 * and "Implementation notes". */

export const BROWSER_STATES = [
  "hibernated",
  "starting",
  "detached",
  "attached",
  "stopping",
] as const;
export type BrowserState = (typeof BROWSER_STATES)[number];
/** Chromium's device scale factor, fixed for the life of an incarnation. */
export type BrowserScale = 1 | 2;

export interface BrowserInstance {
  account_id: string;
  instance_id: string;
  incarnation: number;
}

/** The exact browser the owner is shown: one account may have browsers on
 * several Machines, and every instance-scoped request and the viewer name
 * the Machine and instance (incarnation included) taken from the displayed
 * state, never "whatever runs now". */
export interface BrowserTarget {
  machine_id: string;
  instance: BrowserInstance;
}

/** One Machine observation (`machine_browser::StateEvent`). */
export interface BrowserStateEvent {
  request_id?: string;
  instance: BrowserInstance;
  state: BrowserState;
  reason?: string;
  geometry?: { width: number; height: number };
  scale?: number;
  deadline_ms?: number;
  attachment_id?: string;
  snapshot_generation?: number;
  snapshot_at_ms?: number;
  profile_revision?: number;
  unsaved_state_lost?: boolean;
  observed_at_ms: number;
}

export interface BrowserPlacement {
  account_id: string;
  machine_id: string;
  instance_id: string;
  profile_revision: number;
  last_open_ms: number;
}

/** `GET .../browser`: the latest observation, the durable placements and the
 * admission refusal (`machine_browser_not_enabled`, `browser_not_enabled`,
 * `account_unavailable`), or null when the owner may operate the browser. */
export interface BrowserRead {
  machine_id: string | null;
  state: BrowserStateEvent | null;
  placements: BrowserPlacement[];
  admission: string | null;
  /** The account has no placement and no native credentials anywhere
   * (never signed in): its first open names a Machine the owner chose. */
  choose_machine?: boolean;
  /** Connected Machines that can run a browser now, the choices offered. */
  capable_machines?: { machine_id: string }[];
}

/** Every lifecycle route answers with the Machine's observation. */
export interface BrowserResult {
  machine_id: string;
  state: BrowserStateEvent;
}

export interface BrowserViewport {
  width: number;
  height: number;
  scale: BrowserScale;
}

/** Wire bounds from `components/wire/src/machine_protocol/machine_browser.rs`. */
export const VIEWPORT_LIMITS = { minWidth: 320, minHeight: 480, maxDimension: 4096 } as const;

const ID = /^[A-Za-z0-9_-]{1,128}$/;

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function text(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}
function id(value: unknown): value is string {
  return typeof value === "string" && ID.test(value);
}
function integer(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value);
}
function count(value: unknown): value is number {
  return integer(value) && value >= 0;
}
function optional<T>(
  value: unknown,
  valid: (value: unknown) => value is T,
): value is T | undefined {
  return value === undefined || valid(value);
}

/** Whether an observation names a running or retained instance. The
 * Machine answers a query for a stopped browser and every reset with
 * `instance_id: ""` and `incarnation: 0` (`hibernated`), and a query while an
 * unconfirmed unit of unknown owner exists or its inventory is pending the
 * same way as `stopping` (reason `busy`); neither carries an instance to
 * command. */
export function namesInstance(state: BrowserStateEvent): boolean {
  return state.instance.instance_id !== "";
}

function validInstance(instance: Record<string, unknown>, state: unknown): boolean {
  if (!id(instance.account_id) || !count(instance.incarnation)) return false;
  if (instance.instance_id === "") {
    return (state === "hibernated" || state === "stopping") && instance.incarnation === 0;
  }
  return id(instance.instance_id);
}

export function validBrowserState(value: unknown): value is BrowserStateEvent {
  if (!record(value) || !record(value.instance)) return false;
  const { instance, geometry } = value;
  return (
    validInstance(instance, value.state) &&
    typeof value.state === "string" &&
    (BROWSER_STATES as readonly string[]).includes(value.state) &&
    optional(value.request_id, text) &&
    optional(value.reason, text) &&
    (geometry === undefined ||
      (record(geometry) && count(geometry.width) && count(geometry.height))) &&
    optional(value.scale, count) &&
    optional(value.deadline_ms, integer) &&
    optional(value.attachment_id, text) &&
    optional(value.snapshot_generation, count) &&
    optional(value.snapshot_at_ms, integer) &&
    optional(value.profile_revision, count) &&
    (value.unsaved_state_lost === undefined || typeof value.unsaved_state_lost === "boolean") &&
    integer(value.observed_at_ms)
  );
}

function validPlacement(value: unknown): value is BrowserPlacement {
  return (
    record(value) &&
    id(value.account_id) &&
    text(value.machine_id) &&
    id(value.instance_id) &&
    count(value.profile_revision) &&
    integer(value.last_open_ms)
  );
}

export function validBrowserRead(value: unknown): value is BrowserRead {
  return (
    record(value) &&
    (value.machine_id === null || text(value.machine_id)) &&
    (value.state === null || validBrowserState(value.state)) &&
    (value.admission === null || text(value.admission)) &&
    Array.isArray(value.placements) &&
    value.placements.every(validPlacement) &&
    (value.choose_machine === undefined || typeof value.choose_machine === "boolean") &&
    (value.capable_machines === undefined ||
      (Array.isArray(value.capable_machines) &&
        value.capable_machines.every((machine) => record(machine) && text(machine.machine_id))))
  );
}

export function validBrowserResult(value: unknown): value is BrowserResult {
  return record(value) && text(value.machine_id) && validBrowserState(value.state);
}

/** Framebuffer size for a container measured in CSS pixels: the container
 * times the device scale, held within the wire bounds. */
export function browserViewport(
  cssWidth: number,
  cssHeight: number,
  scale: BrowserScale,
): BrowserViewport {
  const bound = (value: number, min: number): number => {
    const scaled = Number.isFinite(value) ? Math.round(value * scale) : 0;
    return Math.min(VIEWPORT_LIMITS.maxDimension, Math.max(min, scaled));
  };
  return {
    width: bound(cssWidth, VIEWPORT_LIMITS.minWidth),
    height: bound(cssHeight, VIEWPORT_LIMITS.minHeight),
    scale,
  };
}

/** Whether a measured size moved enough in either dimension for a resize. */
export function sizeChanged(
  previous: { width: number; height: number },
  next: { width: number; height: number },
  thresholdPx: number,
): boolean {
  return (
    Math.abs(next.width - previous.width) >= thresholdPx ||
    Math.abs(next.height - previous.height) >= thresholdPx
  );
}

function accountPath(account: string): string {
  return `/api/provider-accounts/${encodeURIComponent(account)}`;
}

/** The viewer WebSocket for exactly `target`; the Controller refuses it
 * (`browser_instance_changed`) once that Machine runs another instance or
 * incarnation. The device proof must sign this exact URL. */
export function browserStreamUrl(
  account: string,
  target: BrowserTarget,
  viewport: BrowserViewport,
  takeover: boolean,
  origin: { protocol: string; host: string } = globalThis.location,
): string {
  const params = new URLSearchParams({
    machine_id: target.machine_id,
    instance_id: target.instance.instance_id,
    incarnation: String(target.instance.incarnation),
    width: String(viewport.width),
    height: String(viewport.height),
    scale: String(viewport.scale),
  });
  if (takeover) params.set("takeover", "true");
  const protocol = origin.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${origin.host}${accountPath(account)}/browser/stream?${params.toString()}`;
}

export class MachineBrowserError extends Error {
  readonly code: string;
  readonly status: number;
  constructor(code: string, status: number) {
    super(browserErrorText(code));
    this.name = "MachineBrowserError";
    this.code = code;
    this.status = status;
  }
}

export type BrowserFetch = (input: string, init: RequestInit) => Promise<Response>;

export interface MachineBrowserApi {
  /** `machine` keeps the read on the Machine the panel shows while that
   * Machine still holds the account's browser. */
  read(account: string, signal: AbortSignal, machine?: string): Promise<BrowserRead>;
  /** `machine` opens only on that Machine (a restart reopens where it
   * stopped); the Controller refuses rather than choose another one. */
  open(
    account: string,
    dataset: string,
    viewport: BrowserViewport,
    signal: AbortSignal,
    machine?: string,
  ): Promise<BrowserResult>;
  /** Navigate exactly `target` to an owner-supplied `https` URL. */
  navigate(
    account: string,
    dataset: string,
    target: BrowserTarget,
    url: string,
    signal: AbortSignal,
  ): Promise<BrowserResult>;
  hibernate(
    account: string,
    dataset: string,
    target: BrowserTarget,
    signal: AbortSignal,
  ): Promise<BrowserResult>;
  snapshot(
    account: string,
    dataset: string,
    target: BrowserTarget,
    signal: AbortSignal,
  ): Promise<BrowserResult>;
  /** Profile revisions are per Machine: the revision is checked only on
   * `machine`, the Machine whose profile the panel shows. */
  reset(
    account: string,
    dataset: string,
    machine: string,
    expectedProfileRevision: number,
    signal: AbortSignal,
  ): Promise<BrowserResult>;
}

/** Only the wire fields: `InstanceRef` rejects unknown keys. */
function targetBody(target: BrowserTarget): { machine_id: string; instance: BrowserInstance } {
  const { instance } = target;
  return {
    machine_id: target.machine_id,
    instance: {
      account_id: instance.account_id,
      instance_id: instance.instance_id,
      incarnation: instance.incarnation,
    },
  };
}

export function createMachineBrowserApi(
  fetchImpl: BrowserFetch = (input, init) => fetch(input, init),
): MachineBrowserApi {
  async function call<T>(
    path: string,
    init: RequestInit,
    signal: AbortSignal,
    valid: (value: unknown) => value is T,
  ): Promise<T> {
    const response = await fetchImpl(path, { ...init, cache: "no-store", signal });
    let body: unknown = null;
    try {
      body = await response.json();
    } catch {
      body = null;
    }
    if (!response.ok) {
      const code =
        record(body) && record(body.error) && text(body.error.code)
          ? body.error.code
          : `http_${response.status}`;
      throw new MachineBrowserError(code, response.status);
    }
    if (!valid(body)) throw new MachineBrowserError("invalid_response", response.status);
    return body;
  }
  const post = (
    path: string,
    body: Record<string, unknown>,
    signal: AbortSignal,
  ): Promise<BrowserResult> =>
    call(
      path,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      },
      signal,
      validBrowserResult,
    );
  return {
    read: (account, signal, machine) =>
      call(
        `${accountPath(account)}/browser${
          machine ? `?${new URLSearchParams({ machine_id: machine }).toString()}` : ""
        }`,
        { method: "GET" },
        signal,
        validBrowserRead,
      ),
    open: (account, dataset, viewport, signal, machine) =>
      post(
        `${accountPath(account)}/browser/open`,
        { dataset, ...viewport, ...(machine ? { machine_id: machine } : {}) },
        signal,
      ),
    navigate: (account, dataset, target, url, signal) =>
      post(
        `${accountPath(account)}/browser/navigate`,
        { dataset, ...targetBody(target), url },
        signal,
      ),
    hibernate: (account, dataset, target, signal) =>
      post(`${accountPath(account)}/browser/hibernate`, { dataset, ...targetBody(target) }, signal),
    snapshot: (account, dataset, target, signal) =>
      post(`${accountPath(account)}/browser/snapshot`, { dataset, ...targetBody(target) }, signal),
    reset: (account, dataset, machine, expectedProfileRevision, signal) =>
      post(
        `${accountPath(account)}/browser/reset`,
        { dataset, machine_id: machine, expected_profile_revision: expectedProfileRevision },
        signal,
      ),
  };
}

export const machineBrowserApi: MachineBrowserApi = createMachineBrowserApi();

const ERROR_TEXT: Record<string, string> = {
  machine_browser_not_enabled: "Machine browsers are not enabled on this Cowboy Service.",
  browser_not_enabled: "The browser is not enabled for this account.",
  account_unavailable: "This account is unavailable.",
  named_account_required: "Only a named account can have a Machine browser.",
  machine_unavailable: "No connected Machine can run this account’s browser right now.",
  browser_host_requirements:
    "The Machine holding this account does not meet the browser host requirements.",
  browser_not_running: "The browser is not running. Open it first.",
  browser_not_placed: "This account's browser lives on the Machine that holds its sign-in, not on this one.",
  browser_attached: "Another viewer is attached to this browser.",
  browser_instance_changed:
    "The browser on this Machine changed since it was shown. Check its current state and retry.",
  browser_attach_refused: "The Machine refused the attachment. Check the browser state and retry.",
  browser_attach_failed: "The Machine did not answer the attachment request.",
  browser_open_failed: "The browser could not be opened on the Machine.",
  browser_machine_unavailable:
    "The browser’s Machine is not connected or cannot run browsers right now; it was not opened elsewhere.",
  browser_restart_unconfirmed:
    "The browser did not confirm that it stopped, so it was not reopened. Check its state and retry.",
  browser_command_failed: "The Machine did not confirm the command.",
  dataset_changed: "Your Cowboy account data changed. Reload before continuing.",
  invalid_viewport: "This screen size is outside the supported range.",
  invalid_argument: "The Service did not accept the request.",
  unavailable: "The Service is temporarily unavailable.",
  resize_failed: "The Machine did not apply the new screen size.",
  account_revoking: "This account is being revoked; its browser profile is deleted with it.",
  invalid_response: "The Service answered in an unexpected shape.",
};

/** Text for an admission or refusal code; HTTP-only failures name the status. */
export function browserErrorText(code: string | null | undefined): string {
  if (code && ERROR_TEXT[code]) return ERROR_TEXT[code];
  const http = code === undefined || code === null ? null : /^http_(\d{3})$/.exec(code);
  if (http) return `The Service answered with HTTP ${http[1]}.`;
  return "The browser request did not succeed.";
}

const REASON_TEXT: Record<string, string> = {
  not_available: "Browsers are not available on this Machine.",
  host_requirements: "This Machine does not meet the browser host requirements.",
  sandbox_unavailable: "The browser sandbox is unavailable on this Machine.",
  confinement_unverified: "The browser’s confinement could not be verified, so it was stopped.",
  listener_detected: "The browser opened a network listener and was stopped.",
  storage_exceeded: "The browser profile exceeded its storage budget.",
  profile_newer_than_runtime:
    "The saved profile was written by a newer browser. Reset the profile to continue.",
  superseded: "A newer instance replaced this one.",
  // Retired sign-in reasons an older Machine may still report.
  login_ended: "An older Machine stopped this browser with its sign-in.",
  login_mismatch: "An older Machine refused this browser for a sign-in.",
  deadline: "The browser reached its maximum running time.",
  grace: "Nobody was attached, so the browser was hibernated.",
  reclaim: "The Machine reclaimed memory and hibernated the browser.",
  revoked: "The account was revoked.",
  disabled: "The browser was disabled for this account.",
  owner_request: "Hibernated on request.",
  capacity: "The Machine already runs its maximum number of browsers.",
  memory: "The Machine has no memory headroom for a browser right now.",
  busy: "Another browser operation for this account is still in progress.",
  failed: "The browser process failed.",
};

/** Text for a lifecycle reason; absent reasons give nothing to show. */
export function browserReasonText(reason: string | null | undefined): string | undefined {
  if (!reason) return undefined;
  return REASON_TEXT[reason] ?? "The Machine reported a reason this Cowboy does not know yet.";
}

const STATE_LABEL: Record<BrowserState, string> = {
  hibernated: "Hibernated",
  starting: "Starting",
  detached: "Running, not attached",
  attached: "Attached",
  stopping: "Stopping",
};

export function browserStateLabel(state: BrowserState | null | undefined): string {
  return state ? STATE_LABEL[state] : "Never opened";
}
