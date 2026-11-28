import type { DeliveryTarget } from "./currentTurnDelivery";
import type { DeliveryOptions } from "./deliveryPolicy";
import type { SessionMeta, WireDeliveryOrigin } from "./protocol";

export interface WrapUpOutboxFields {
  wrapUp?: boolean;
  wrapUpSessionId?: string;
  wrapUpTarget?: DeliveryTarget;
}

/** True when a staged row is Owner-authored (or freshly authored locally
 *  before server round-trip). Confirmed server rows without C-auth `owner`
 *  provenance (Peer, Automation, Cowboy, or legacy pre-C-auth) return false. */
export function isOwnerAuthoredRow(
  row: { readonly status?: unknown; readonly serverOrigin?: WireDeliveryOrigin } | null | undefined,
): boolean {
  if (!row) return true;
  if (row.serverOrigin !== undefined) return row.serverOrigin.kind === "owner";
  return row.status !== undefined;
}

export interface DeliveryPickerStateInput {
  busy: boolean;
  paused: boolean;
  queued: boolean;
  stagedDraft?: boolean;
  queueCount?: number;
  steerReady: boolean;
  wrapUpReady: boolean;
  contextReady: boolean;
  ownerAuthored?: boolean;
  editingRowId?: string | null;
  editingToken?: string | null;
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

/** Resolve DeliveryPicker row availability and initial focus. Antigravity
 *  wrap-up steering activates when native steer is unavailable and every
 *  precondition holds; queued/draft rows enforce C-auth Owner provenance and
 *  require Controller queued-wrap-up support (`wrap_up_queued_supported`). */
export function resolveDeliveryPickerState(
  input: DeliveryPickerStateInput,
): DeliveryPickerState {
  const {
    busy,
    paused,
    queued,
    stagedDraft = false,
    queueCount,
    steerReady,
    wrapUpReady,
    contextReady,
    ownerAuthored = true,
    editingRowId = null,
    editingToken = null,
    disallowSteer = false,
    fresh,
    error,
    options,
  } = input;
  // Use cached options only for capability hints; pause/editing flags from a
  // prior opening must never shadow live props on reopen before fresh read.
  const effectivePaused = paused || (fresh && Boolean(options?.paused));
  const selfEditing = Boolean(
    editingRowId && editingToken &&
      options?.editing_id === editingRowId &&
      options?.editing_token === editingToken,
  );
  const effectiveEditing = fresh && Boolean(options?.editing) && !selfEditing;
  const target = fresh && busy ? options?.target ?? undefined : undefined;
  const steer = disallowSteer
    ? false
    : fresh
    ? Boolean(busy && options?.steer && target)
    : Boolean(busy && steerReady);
  const checkpoint = !disallowSteer && steer && Boolean(options?.controls.next_checkpoint);
  const stagedWrapUpAvailable = queued
    ? Boolean(options?.wrap_up_queued_supported)
    : stagedDraft
    ? Boolean(options?.wrap_up_draft_supported)
    : true;
  const showWrapUp = !steer && (
    fresh
      ? Boolean(!options?.steer && options?.wrap_up_steering && stagedWrapUpAvailable)
      : Boolean(
        wrapUpReady ||
          (!options?.steer && options?.wrap_up_steering && stagedWrapUpAvailable),
      )
  );
  // Require a fresh read with an exact target before enabling wrap-up so the
  // owner never hints a later turn than the one shown when the picker opened.
  const wrapUp = showWrapUp && ownerAuthored && busy && !effectivePaused &&
    !effectiveEditing && contextReady && fresh && Boolean(target);
  const unavailable = !busy
    ? "No active turn"
    : disallowSteer || (fresh && !options?.steer)
    ? "Not supported by this worker"
    : fresh
    ? "Active turn unavailable"
    : error
    ? "Runtime unavailable"
    : "Checking runtime…";
  const wrapUpUnavailable = !ownerAuthored
    ? "Only your own messages can request wrap-up"
    : !busy
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
  const frontAlreadyFirst = queued && queueCount === 0;
  // Only Antigravity wrap-up overrides an already-first disabled Queue first
  // row under non-steer policies; native Codex/Claude Steer keeps respecting
  // next_turn so Enter never injects unexpectedly into an active turn.
  const preferred = mode === "tool_boundary"
    ? "checkpoint"
    : mode === "steer" || (frontAlreadyFirst && showWrapUp)
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

/** Rollback-safe wire encoding for activating an Owner draft with Antigravity
 *  wrap-up steering; omits wrap-up fields on Controllers without support. */
export function wrapUpActivateFields(
  sessions: readonly Pick<SessionMeta, "id" | "wrap_up_steering">[],
  sessionId: string,
  activation: WrapUpOutboxFields & { readonly front?: boolean; readonly editingToken?: string },
  draftWrapUpSupported = true,
): { wrap_up?: true; wrap_up_target?: DeliveryTarget; editing_token?: string } {
  if (
    !draftWrapUpSupported ||
    !activation.front ||
    activation.wrapUp !== true ||
    !activation.wrapUpTarget
  ) {
    return {};
  }
  if (activation.wrapUpSessionId !== undefined && activation.wrapUpSessionId !== sessionId) {
    return {};
  }
  const session = sessions.find((candidate) => candidate.id === sessionId);
  if (session?.wrap_up_steering !== true) {
    return {};
  }
  return {
    wrap_up: true,
    wrap_up_target: activation.wrapUpTarget,
    ...(activation.editingToken ? { editing_token: activation.editingToken } : {}),
  };
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

export function isRecoveryDraftId(id: string): boolean {
  return id.startsWith("recovery-");
}

export async function postWrapUpQueued(
  sessionId: string,
  rowId: string,
  target?: DeliveryTarget,
  editingToken?: string,
): Promise<{ status: "queued" | "dispatched"; hinted: boolean }> {
  const payload = {
    ...(target ? { target } : {}),
    ...(editingToken ? { editing_token: editingToken } : {}),
  };
  const response = await fetch(
    `/api/sessions/${encodeURIComponent(sessionId)}/queue/${encodeURIComponent(rowId)}/wrap-up`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
      signal: AbortSignal.timeout(5000),
    },
  );
  if (!response.ok) {
    const message = (await response.text()).trim();
    throw new Error(message || "Could not request wrap-up for this message.");
  }
  const body = await response.json() as { status?: string; hinted?: boolean };
  return {
    status: body.status === "dispatched" ? "dispatched" : "queued",
    hinted: body.hinted === true,
  };
}

export function bulkActivatableDrafts<T extends { readonly id: string }>(
  drafts: readonly T[],
): T[] {
  return drafts.filter((draft) => !isRecoveryDraftId(draft.id));
}
