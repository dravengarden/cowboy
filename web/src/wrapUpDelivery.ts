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
    fresh,
    error,
    options,
  } = input;
  const target = fresh && busy ? options?.target ?? undefined : undefined;
  const steer = fresh
    ? Boolean(busy && options?.steer && target)
    : Boolean(busy && steerReady);
  const checkpoint = steer && Boolean(options?.controls.next_checkpoint);
  const showWrapUp = !queued && !steer && (
    fresh
      ? Boolean(!options?.steer && options?.wrap_up_steering)
      : Boolean(wrapUpReady || (!options?.steer && options?.wrap_up_steering))
  );
  const wrapUp = showWrapUp && busy && !paused && !options?.editing &&
    contextReady && (fresh ? Boolean(target) : Boolean(wrapUpReady));
  const unavailable = !busy
    ? "No active turn"
    : fresh && !options?.steer
    ? "Not supported by this worker"
    : fresh
    ? "Active turn unavailable"
    : error
    ? "Runtime unavailable"
    : "Checking runtime…";
  const wrapUpUnavailable = !busy
    ? "No active turn"
    : paused
    ? "Queue is paused"
    : options?.editing
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

export async function captureWrapUpTarget(
  sessionId: string,
  target?: DeliveryTarget,
): Promise<DeliveryTarget | undefined> {
  if (target !== undefined) return target;
  const { captureDeliveryTarget } = await import("./currentTurnDelivery");
  return await captureDeliveryTarget(sessionId).catch(() => undefined);
}
