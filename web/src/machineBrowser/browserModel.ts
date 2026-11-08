/** Pure state of the Machine browser panel: the last Controller read, the
 * action in flight, whether the viewer is mounted and the remote clipboard
 * held for the open attachment. The hook feeds it events; the sheet renders it. */
import {
  type BrowserRead,
  type BrowserResult,
  type BrowserState,
  type BrowserStateEvent,
  type BrowserTarget,
  MachineBrowserError,
  browserReasonText,
  namesInstance,
} from "./api";

export type BrowserAction = "open" | "hibernate" | "snapshot" | "reset";

export interface BrowserModel {
  read?: BrowserRead | undefined;
  /** The first read answered (successfully or not). */
  loaded: boolean;
  error: string;
  notice: string;
  busy?: BrowserAction | undefined;
  /** The viewer mounts while this is set; `generation` remounts it. It
   * attaches to exactly `target`, never to another browser of the account.
   * `since` is the lifecycle epoch it was established in. */
  viewer?:
    | { takeover: boolean; generation: number; target: BrowserTarget; since?: number }
    | undefined;
  /** Text the remote browser copied, held for the open attachment only. */
  remoteClipboard?: string | undefined;
  /** When the instance was first seen `starting`, bounding the fast poll. */
  startingSince?: number | undefined;
  /** An open answered `starting` for this instance; attach to it once it
   * runs. */
  awaitingOpen?: BrowserTarget | undefined;
  /** Advances on every lifecycle transition (an applied command result, a
   * viewer mounted or ended, an awaited open settled); a read issued before
   * the current epoch describes an earlier lifecycle and is discarded. */
  epoch?: number;
}

export type BrowserEvent =
  | {
      type: "read";
      read: BrowserRead;
      now: number;
      /** The lifecycle epoch the read was issued in (`readTicket`). */
      issued?: number | undefined;
    }
  | { type: "read_failed"; message: string; issued?: number | undefined }
  | { type: "busy"; action: BrowserAction }
  | {
      type: "result";
      action: BrowserAction;
      result: BrowserResult;
      now: number;
      /** An open asked to take over an attached display (sign-in entry). */
      takeover?: boolean;
      /** The browser the owner confirmed taking over; another answer never
       * takes a display over. */
      expected?: BrowserTarget | undefined;
    }
  | { type: "failed"; action: BrowserAction; message: string }
  | {
      type: "attach";
      takeover: boolean;
      /** The browser the owner confirmed; nothing attaches once the panel
       * shows another one. */
      target?: BrowserTarget | undefined;
    }
  | { type: "detach" }
  | { type: "stream_state"; state: BrowserStateEvent }
  | { type: "viewer_ended"; message: string }
  | { type: "remote_clipboard"; text: string | undefined }
  | { type: "notice"; message: string }
  | { type: "dismiss" };

export const INITIAL_BROWSER_MODEL: BrowserModel = { loaded: false, error: "", notice: "" };

/** The Machine and instance a read or result shows, when it names one. */
export function targetOf(
  read: { machine_id: string | null; state: BrowserStateEvent | null } | undefined,
): BrowserTarget | undefined {
  const state = read?.state;
  if (!read?.machine_id || !state || !namesInstance(state)) return undefined;
  const { account_id, instance_id, incarnation } = state.instance;
  return { machine_id: read.machine_id, instance: { account_id, instance_id, incarnation } };
}

export function sameTarget(left: BrowserTarget, right: BrowserTarget | undefined): boolean {
  return (
    right !== undefined &&
    left.machine_id === right.machine_id &&
    left.instance.instance_id === right.instance.instance_id &&
    left.instance.incarnation === right.instance.incarnation
  );
}

/** The Machine and instance every instance command names: the mounted
 * viewer's while one is mounted, else the browser the panel shows. */
export function shownTarget(model: BrowserModel): BrowserTarget | undefined {
  return model.viewer?.target ?? targetOf(model.read);
}

/** Whether a read describes something other than the mounted viewer's
 * browser and must not replace it: another Machine's answer (a poll that
 * finished after an open mounted the viewer elsewhere), an older
 * incarnation on the viewer's Machine, or an answer naming no instance
 * that was issued before the viewer's instance was established (it may
 * describe the browser a restart stopped). A newer instance there, or a
 * later answer that no instance runs, describes the viewer's browser
 * ending. */
export function staleForViewer(model: BrowserModel, read: BrowserRead, issued?: number): boolean {
  const viewer = model.viewer;
  if (!viewer) return false;
  if (read.machine_id !== viewer.target.machine_id) return true;
  const state = read.state;
  if (state === null || !namesInstance(state)) {
    return issued !== undefined && issued < (viewer.since ?? 0);
  }
  return state.instance.incarnation < viewer.target.instance.incarnation;
}

/** Whether a read was issued in an earlier lifecycle epoch than the panel's. */
export function issuedBefore(model: BrowserModel, issued: number | undefined): boolean {
  return issued !== undefined && issued < (model.epoch ?? 0);
}

/** The Machine the panel's reads stay on: the browser it is attached to or
 * waiting for, else the one it shows while that is still live. Once that
 * browser is gone, reads report the account's preferred browser again. */
export function pinnedMachine(model: BrowserModel): string | undefined {
  if (model.viewer) return model.viewer.target.machine_id;
  if (model.awaitingOpen) return model.awaitingOpen.machine_id;
  const state = model.read?.state?.state;
  return state && state !== "hibernated" ? (model.read?.machine_id ?? undefined) : undefined;
}

/** Tags one state read with its generation and the Machine it is pinned to.
 * Its answer applies only while it is still the newest read and the panel
 * is still pinned there: a poll for another Machine that finishes after an
 * open mounted the viewer is obsolete. */
export function readTicket(
  generations: { current: number },
  model: () => BrowserModel,
): { pinned: string | undefined; epoch: number; current: () => boolean } {
  const generation = ++generations.current;
  const issued = model();
  const pinned = pinnedMachine(issued);
  const epoch = issued.epoch ?? 0;
  return {
    pinned,
    epoch,
    current: () =>
      generation === generations.current &&
      pinned === pinnedMachine(model()) &&
      epoch === (model().epoch ?? 0),
  };
}

/** Whether a Machine's answer confirms that exactly `target` stopped:
 * hibernated on its Machine for that instance and incarnation, or that
 * Machine's answer that no instance runs. */
export function stoppedFor(
  target: BrowserTarget,
  answer: { machine_id: string | null; state: BrowserStateEvent | null },
): boolean {
  const state = answer.state;
  if (answer.machine_id !== target.machine_id || state?.state !== "hibernated") return false;
  if (!namesInstance(state)) return true;
  return (
    state.instance.instance_id === target.instance.instance_id &&
    state.instance.incarnation === target.instance.incarnation
  );
}

export const RESTART_CONFIRM_INTERVAL_MS = 1_000;
export const RESTART_CONFIRM_ATTEMPTS = 30;

function pause(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(signal.reason);
      return;
    }
    const abort = (): void => {
      clearTimeout(timer);
      reject(signal.reason);
    };
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", abort);
      resolve();
    }, ms);
    signal.addEventListener("abort", abort, { once: true });
  });
}

/** Wait until `target` verifiably stopped on its Machine before a restart
 * reopens there: the hibernate's own answer, else reads pinned to that
 * Machine (bounded). A newer instance there, or no confirmation in time,
 * refuses the restart. */
export async function confirmStopped(
  target: BrowserTarget,
  hibernated: BrowserResult,
  read: (signal: AbortSignal) => Promise<BrowserRead>,
  signal: AbortSignal,
  wait: (ms: number, signal: AbortSignal) => Promise<void> = pause,
): Promise<void> {
  let answer: { machine_id: string | null; state: BrowserStateEvent | null } = hibernated;
  for (let attempt = 0; ; attempt++) {
    if (stoppedFor(target, answer)) return;
    const state = answer.state;
    if (
      answer.machine_id === target.machine_id &&
      state !== null &&
      namesInstance(state) &&
      state.instance.incarnation > target.instance.incarnation
    ) {
      throw new MachineBrowserError("browser_instance_changed", 409);
    }
    if (attempt >= RESTART_CONFIRM_ATTEMPTS) {
      throw new MachineBrowserError("browser_restart_unconfirmed", 409);
    }
    await wait(RESTART_CONFIRM_INTERVAL_MS, signal);
    answer = await read(signal);
  }
}

/** The Machine and profile revision a reset applies to, taken together from
 * the state shown (or, without one, the placement shown): revisions are per
 * Machine and must never be checked against another Machine's profile. */
export function resetTarget(
  read: BrowserRead | undefined,
): { machine_id: string; revision: number } | undefined {
  if (read?.machine_id && read.state?.profile_revision !== undefined) {
    return { machine_id: read.machine_id, revision: read.state.profile_revision };
  }
  const placement = read?.machine_id
    ? read.placements.find((candidate) => candidate.machine_id === read.machine_id)
    : read?.placements[0];
  return placement
    ? { machine_id: placement.machine_id, revision: placement.profile_revision }
    : undefined;
}

export const POLL_IDLE_MS = 5_000;
export const POLL_STARTING_MS = 2_000;
/** After this long in `starting`, polling returns to the idle period. */
export const STARTING_FAST_POLL_MS = 180_000;

export function browserRunning(state: BrowserState | null | undefined): boolean {
  return state === "detached" || state === "attached";
}

/** A display exists only for a running instance that was not replaced. */
export function viewerEnds(state: BrowserStateEvent): boolean {
  return !browserRunning(state.state) || state.reason === "superseded";
}

/** Whether an open was refused although the Machine answered with a live
 * instance: reusing a running browser for a sign-in answers with that
 * browser's own state, and a reason on it (`login_mismatch`, `busy`,
 * `failed`, ...) means the sign-in was not bound. Such an answer never
 * mounts the viewer or takes the display over. */
export function openRefused(state: BrowserStateEvent): boolean {
  return state.state !== "hibernated" && state.reason !== undefined;
}

/** How the panel enters a browser it was asked to show at once. A sign-in
 * panel always goes through the sign-in open, even for a running browser,
 * so the Machine validates and binds the login before anything attaches; a
 * tab panel attaches to a detached browser directly. An attached display is
 * never entered without the owner's takeover. */
export function autoEntry(
  signIn: boolean,
  state: BrowserState | null | undefined,
): "open" | "attach" | undefined {
  if (!state || state === "hibernated") return "open";
  if (state === "detached") return signIn ? "open" : "attach";
  return undefined;
}

/** What a finished snapshot or reset means for the owner: the success text,
 * or the failure the Machine reported (HTTP 200 carries both). */
export function operationOutcome(
  action: "snapshot" | "reset",
  state: BrowserStateEvent,
): { notice: string } | { error: string } {
  if (action === "snapshot") {
    if (!state.reason) return { notice: "Snapshot published." };
    return {
      error: `The snapshot was not published. ${browserReasonText(state.reason) ?? ""}`.trim(),
    };
  }
  if (state.state === "hibernated" && state.reason === "owner_request") {
    return { notice: "The profile was reset." };
  }
  if (state.reason === "superseded") {
    return { notice: "The profile changed elsewhere; nothing was reset." };
  }
  return {
    error: `The profile was not reset. ${browserReasonText(state.reason) ?? "The Machine did not confirm the reset."}`,
  };
}

/** The remote clipboard text the sign-in dialog may offer as an authorization
 * code: one non-empty token without whitespace, within the server's bound. */
export function looksLikeAuthorizationCode(text: string): boolean {
  const value = text.trim();
  return value.length > 0 && value.length <= 4096 && !/\s/.test(value);
}

export const CHANGED_BEFORE_CONFIRM =
  "The browser shown changed before you confirmed; nothing was done.";

const EMPTY_READ: BrowserRead = { machine_id: null, state: null, placements: [], admission: null };

function nextGeneration(model: BrowserModel): number {
  return (model.viewer?.generation ?? 0) + 1;
}

function endNotice(state: BrowserStateEvent): string {
  return (
    browserReasonText(state.reason) ??
    (state.state === "stopping" ? "The browser is stopping." : "The browser was hibernated.")
  );
}

/** Merge a fresh observation, keeping the fast-poll clock and the viewer
 * consistent with what the Machine now reports. */
function withRead(model: BrowserModel, read: BrowserRead, now: number): BrowserModel {
  const state = read.state;
  const starting = state?.state === "starting";
  const next: BrowserModel = {
    ...model,
    read,
    loaded: true,
    startingSince: starting ? (model.startingSince ?? now) : undefined,
  };
  // The viewer ends with its instance: stopped, superseded, or replaced by
  // a newer instance on its Machine (`staleForViewer` drops older ones).
  const replaced =
    model.viewer !== undefined &&
    state !== null &&
    namesInstance(state) &&
    !sameTarget(model.viewer.target, targetOf(read));
  if (model.viewer && state && (viewerEnds(state) || replaced)) {
    return {
      ...next,
      viewer: undefined,
      remoteClipboard: undefined,
      notice: endNotice(state),
    };
  }
  // Reads stay on the awaited Machine (`pinnedMachine`); another Machine's
  // answer means the awaited browser is gone, never a browser to attach.
  const awaiting = model.awaitingOpen;
  if (awaiting && state && (!starting || read.machine_id !== awaiting.machine_id)) {
    if (!sameTarget(awaiting, targetOf(read))) {
      return {
        ...next,
        awaitingOpen: undefined,
        error: "The browser on this Machine changed before it finished starting.",
      };
    }
    if (state.state === "detached") {
      return {
        ...next,
        awaitingOpen: undefined,
        viewer: { takeover: false, generation: nextGeneration(model), target: awaiting },
      };
    }
    return {
      ...next,
      awaitingOpen: undefined,
      error:
        state.state === "attached"
          ? ""
          : (browserReasonText(state.reason) ?? "The browser did not start."),
    };
  }
  return next;
}

/** Every lifecycle transition advances the epoch; a viewer established by
 * it records that epoch. */
export function reduceBrowser(model: BrowserModel, event: BrowserEvent): BrowserModel {
  const next = reduceEvent(model, event);
  const transition =
    event.type === "result" ||
    next.viewer !== model.viewer ||
    next.awaitingOpen !== model.awaitingOpen;
  if (!transition) return next;
  const epoch = (model.epoch ?? 0) + 1;
  const viewer =
    next.viewer && next.viewer !== model.viewer ? { ...next.viewer, since: epoch } : next.viewer;
  return { ...next, epoch, viewer };
}

function reduceEvent(model: BrowserModel, event: BrowserEvent): BrowserModel {
  switch (event.type) {
    case "read":
      if (issuedBefore(model, event.issued)) return model;
      if (staleForViewer(model, event.read, event.issued)) return { ...model, loaded: true };
      return withRead(model, event.read, event.now);
    case "read_failed":
      if (issuedBefore(model, event.issued)) return model;
      return { ...model, loaded: true, error: event.message };
    case "busy":
      return { ...model, busy: event.action, error: "", notice: "" };
    case "result": {
      const read: BrowserRead = {
        ...(model.read ?? EMPTY_READ),
        machine_id: event.result.machine_id,
        state: event.result.state,
      };
      const next = withRead({ ...model, busy: undefined }, read, event.now);
      const state = event.result.state;
      // The viewer attaches to exactly the instance this answer names.
      const target = targetOf(read);
      if (event.action === "open") {
        if (openRefused(state)) {
          return {
            ...next,
            awaitingOpen: undefined,
            error: `The browser was not opened. ${browserReasonText(state.reason) ?? ""}`.trim(),
          };
        }
        if (state.state === "detached" && target) {
          return {
            ...next,
            viewer: { takeover: false, generation: nextGeneration(model), target },
          };
        }
        if (state.state === "attached" && event.takeover && target) {
          if (event.expected && !sameTarget(event.expected, target)) {
            return { ...next, error: CHANGED_BEFORE_CONFIRM };
          }
          return { ...next, viewer: { takeover: true, generation: nextGeneration(model), target } };
        }
        if (state.state === "starting" && target) return { ...next, awaitingOpen: target };
        if (state.state === "hibernated") {
          return {
            ...next,
            error: browserReasonText(state.reason) ?? "The browser did not start.",
          };
        }
      }
      if (event.action === "snapshot" || event.action === "reset") {
        return { ...next, ...operationOutcome(event.action, state) };
      }
      return next;
    }
    case "failed":
      return { ...model, busy: undefined, awaitingOpen: undefined, error: event.message };
    case "attach": {
      // Attach to the browser the panel shows, by Machine and instance.
      const target = targetOf(model.read);
      if (!target) return model;
      if (event.target && !sameTarget(event.target, target)) {
        return { ...model, error: CHANGED_BEFORE_CONFIRM };
      }
      return {
        ...model,
        viewer: { takeover: event.takeover, generation: nextGeneration(model), target },
        remoteClipboard: undefined,
        error: "",
        notice: "",
      };
    }
    case "detach":
      return { ...model, viewer: undefined, remoteClipboard: undefined };
    case "stream_state": {
      // The stream reports the instance it is attached to, on its Machine.
      const read: BrowserRead = {
        ...(model.read ?? EMPTY_READ),
        machine_id: model.viewer?.target.machine_id ?? model.read?.machine_id ?? null,
        state: event.state,
      };
      return withRead(model, read, event.state.observed_at_ms);
    }
    case "viewer_ended":
      return { ...model, viewer: undefined, remoteClipboard: undefined, notice: event.message };
    case "remote_clipboard":
      return { ...model, remoteClipboard: event.text };
    case "notice":
      return { ...model, notice: event.message };
    case "dismiss":
      return { ...model, error: "", notice: "" };
    default:
      return model;
  }
}

/** How often the panel re-reads the state, or `undefined` while a stream is
 * attached and reports state itself. */
export function pollPeriod(model: BrowserModel, now: number): number | undefined {
  if (model.viewer) return undefined;
  const starting = model.read?.state?.state === "starting";
  if (
    (starting || model.awaitingOpen) &&
    model.startingSince !== undefined &&
    now - model.startingSince < STARTING_FAST_POLL_MS
  ) {
    return POLL_STARTING_MS;
  }
  return POLL_IDLE_MS;
}
