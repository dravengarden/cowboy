import type { DeliveryTarget } from "./currentTurnDelivery";
import type { DeliveryOptions } from "./deliveryPolicy";
import type { SessionMeta } from "./protocol";

export interface WrapUpOutboxFields {
  wrapUp?: boolean;
  wrapUpSessionId?: string;
  wrapUpTarget?: DeliveryTarget;
}

export interface DeliveryPickerStateInput {
  busy: boolean;
  paused: boolean;
  queued: boolean;
  steerReady: boolean;
  wrapUpReady: boolean;
  contextReady: boolean;
  disallowSteer?: boolean;
  fresh: boolean;
  error: string | null;
  options: DeliveryOptions | null;
}

export interface DeliveryPickerState {
  target: DeliveryTarget | undefined;
  steer: boolean;
  checkpoint: boolean;
  showWrapUp: boolean;
  wrapUp: boolean;
  unavailable: string;
  wrapUpUnavailable: string;
  preferred: "steer" | "wrap_up" | "checkpoint" | "front" | "next";
}

/** Resolve DeliveryPicker row availability and initial focus. Composer-only
 *  wrap-up steering activates exclusively when native steer is unavailable and
 *  every R1 precondition holds; queued rows never promote Peer/Automation text. */
export function resolveDeliveryPickerState(
  input: DeliveryPickerStateInput,
): DeliveryPickerState {
  const {
    busy,
    paused,
    queued,
    steerReady,
    wrapUpReady,
    contextReady,
    disallowSteer = false,
    fresh,
    error,
    options,
  } = input;
  // Use cached options only for capability hints; pause/editing flags from a
  // prior opening must never shadow live props on reopen before fresh read.
  const effectivePaused = paused || (fresh && Boolean(options?.paused));
  const effectiveEditing = fresh && Boolean(options?.editing);
  const target = fresh && busy ? options?.target ?? undefined : undefined;
  const steer = disallowSteer
    ? false
    : fresh
    ? Boolean(busy && options?.steer && target)
    : Boolean(busy && steerReady);
  const checkpoint = !disallowSteer && steer && Boolean(options?.controls.next_checkpoint);
  const showWrapUp = !queued && !steer && (
    fresh
      ? Boolean(!options?.steer && options?.wrap_up_steering)
      : Boolean(wrapUpReady || (!options?.steer && options?.wrap_up_steering))
  );
  // Require a fresh read with an exact target before enabling wrap-up so the
  // owner never hints a later turn than the one shown when the picker opened.
  const wrapUp = showWrapUp && busy && !effectivePaused && !effectiveEditing &&
    contextReady && fresh && Boolean(target);
  const unavailable = !busy
    ? "No active turn"
    : disallowSteer || (fresh && !options?.steer)
    ? "Not supported by this worker"
    : fresh
    ? "Active turn unavailable"
    : error
    ? "Runtime unavailable"
    : "Checking runtime…";
  const wrapUpUnavailable = !busy
    ? "No active turn"
    : effectivePaused
    ? "Queue is paused"
    : effectiveEditing
    ? "Finish editing queued message first"
    : !contextReady
    ? "Pending context change"
    : fresh
    ? "Active turn unavailable"
    : error
    ? "Runtime unavailable"
    : "Checking runtime…";
  const mode = options?.policy.mode ?? "steer";
  const preferred = mode === "tool_boundary"
    ? "checkpoint"
    : mode === "steer"
    ? (showWrapUp ? "wrap_up" : "steer")
    : queued
    ? "front"
    : "next";
  return {
    target,
    steer,
    checkpoint,
    showWrapUp,
    wrapUp,
    unavailable,
    wrapUpUnavailable,
    preferred,
  };
}

/** Rollback-safe wire encoding for a durable front-queue outbox mutation:
 *  attaches `wrap_up` + `wrap_up_target` ONLY when the bound session matches
 *  and the active Controller advertises `wrap_up_steering === true`; otherwise
 *  omits both fields so pre-R1 Controllers accept the same `cmid` cleanly. */
export function wrapUpSubmitFields(
  sessions: readonly Pick<SessionMeta, "id" | "wrap_up_steering">[],
  sessionId: string,
  mutatorName: string,
  row: WrapUpOutboxFields,
): { wrap_up?: true; wrap_up_target?: DeliveryTarget } {
  if (mutatorName !== "frontQueue" || row.wrapUp !== true || !row.wrapUpTarget) {
    return {};
  }
  if (row.wrapUpSessionId !== undefined && row.wrapUpSessionId !== sessionId) {
    return {};
  }
  const session = sessions.find((candidate) => candidate.id === sessionId);
  if (session?.wrap_up_steering !== true) {
    return {};
  }
  return { wrap_up: true, wrap_up_target: row.wrapUpTarget };
}

export function wrapUpOutboxPatch(opts: WrapUpOutboxFields): WrapUpOutboxFields {
  if (opts.wrapUp !== true) return {};
  return {
    wrapUp: true,
    ...(opts.wrapUpSessionId !== undefined ? { wrapUpSessionId: opts.wrapUpSessionId } : {}),
    ...(opts.wrapUpTarget !== undefined ? { wrapUpTarget: opts.wrapUpTarget } : {}),
  };
}

/** Build qAdd options for wrap-up: persist the exact target chosen in the
 *  picker; never re-capture a later turn, and downgrade targetless submits to
 *  plain front-of-queue admission without a hint. */
export function wrapUpQueueAddOptions(
  sessionId: string,
  target?: DeliveryTarget,
): { mode: "front"; origin: "composer" } & WrapUpOutboxFields {
  if (!target) {
    return { mode: "front", origin: "composer" };
  }
  return {
    mode: "front",
    origin: "composer",
    wrapUp: true,
    wrapUpSessionId: sessionId,
    wrapUpTarget: target,
  };
}
