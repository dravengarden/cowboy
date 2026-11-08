/** Pure state of the Machine browser panel: the last Controller read, the
 * action in flight, whether the viewer is mounted and the remote clipboard
 * held for the open attachment. The hook feeds it events; the sheet renders it. */
import {
  type BrowserRead,
  type BrowserResult,
  type BrowserState,
  type BrowserStateEvent,
  browserReasonText,
} from "./api";

export type BrowserAction = "open" | "hibernate" | "snapshot" | "reset";

export interface BrowserModel {
  read?: BrowserRead | undefined;
  /** The first read answered (successfully or not). */
  loaded: boolean;
  error: string;
  notice: string;
  busy?: BrowserAction | undefined;
  /** The viewer mounts while this is set; `generation` remounts it. */
  viewer?: { takeover: boolean; generation: number } | undefined;
  /** Text the remote browser copied, held for the open attachment only. */
  remoteClipboard?: string | undefined;
  /** When the instance was first seen `starting`, bounding the fast poll. */
  startingSince?: number | undefined;
  /** An open answered `starting`; attach once the instance is running. */
  awaitingOpen?: boolean | undefined;
}

export type BrowserEvent =
  | { type: "read"; read: BrowserRead; now: number }
  | { type: "read_failed"; message: string }
  | { type: "busy"; action: BrowserAction }
  | {
      type: "result";
      action: BrowserAction;
      result: BrowserResult;
      now: number;
      /** An open asked to take over an attached display (sign-in entry). */
      takeover?: boolean;
    }
  | { type: "failed"; action: BrowserAction; message: string }
  | { type: "attach"; takeover: boolean }
  | { type: "detach" }
  | { type: "stream_state"; state: BrowserStateEvent }
  | { type: "viewer_ended"; message: string }
  | { type: "remote_clipboard"; text: string | undefined }
  | { type: "notice"; message: string }
  | { type: "dismiss" };

export const INITIAL_BROWSER_MODEL: BrowserModel = { loaded: false, error: "", notice: "" };

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
  if (model.viewer && state && viewerEnds(state)) {
    return {
      ...next,
      viewer: undefined,
      remoteClipboard: undefined,
      notice: endNotice(state),
    };
  }
  if (model.awaitingOpen && state && !starting) {
    if (state.state === "detached") {
      return {
        ...next,
        awaitingOpen: undefined,
        viewer: { takeover: false, generation: nextGeneration(model) },
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

export function reduceBrowser(model: BrowserModel, event: BrowserEvent): BrowserModel {
  switch (event.type) {
    case "read":
      return withRead(model, event.read, event.now);
    case "read_failed":
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
      if (event.action === "open") {
        if (state.state === "detached") {
          return { ...next, viewer: { takeover: false, generation: nextGeneration(model) } };
        }
        if (state.state === "attached" && event.takeover) {
          return { ...next, viewer: { takeover: true, generation: nextGeneration(model) } };
        }
        if (state.state === "starting") return { ...next, awaitingOpen: true };
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
    case "attach":
      return {
        ...model,
        viewer: { takeover: event.takeover, generation: nextGeneration(model) },
        remoteClipboard: undefined,
        error: "",
        notice: "",
      };
    case "detach":
      return { ...model, viewer: undefined, remoteClipboard: undefined };
    case "stream_state": {
      const read: BrowserRead = { ...(model.read ?? EMPTY_READ), state: event.state };
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
