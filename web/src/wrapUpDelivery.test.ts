import { describe, expect, test } from "bun:test";
import { isCurrentContextReady, type SessionContext } from "./sessionContext";
import {
  bulkActivatableDrafts,
  isOwnerAuthoredRow,
  isRecoveryDraftId,
  resolveDeliveryPickerState,
  wrapUpActivateFields,
  wrapUpQueueAddOptions,
  wrapUpSubmitFields,
} from "./wrapUpDelivery";

const baseOptions = {
  target: { worker_epoch: "w1", turn_id: "t1" },
  steer: false,
  wrap_up_steering: true,
  wrap_up_queued_supported: true,
  wrap_up_draft_supported: true,
  editing: false,
  editing_id: null as string | null,
  editing_token: null as string | null,
  controls: { version: 1, next_checkpoint: false },
  status: "busy",
  paused: false,
  hold: null,
  policy: { mode: "steer" as const, revision: 0 },
};

describe("resolveDeliveryPickerState (Antigravity R1 wrap-up)", () => {
  test("enables Wrap up & send next for Composer when all R1 preconditions hold", () => {
    const state = resolveDeliveryPickerState({
      busy: true,
      paused: false,
      queued: false,
      steerReady: false,
      wrapUpReady: true,
      contextReady: true,
      fresh: true,
      error: null,
      options: baseOptions,
    });
    expect(state.showWrapUp).toBe(true);
    expect(state.wrapUp).toBe(true);
    expect(state.steer).toBe(false);
    expect(state.preferred).toBe("wrap_up");
  });

  test("disables Wrap up & send next when unloaded, paused, editing, targetless, idle, or context pending", () => {
    for (const patch of [
      { fresh: false },
      { paused: true },
      { options: { ...baseOptions, paused: true } },
      { busy: false },
      { contextReady: false },
      { options: { ...baseOptions, editing: true } },
      { options: { ...baseOptions, target: null } },
    ]) {
      const state = resolveDeliveryPickerState({
        busy: true,
        paused: false,
        queued: false,
        steerReady: false,
        wrapUpReady: true,
        contextReady: true,
        fresh: true,
        error: null,
        options: baseOptions,
        ...patch,
      });
      expect(state.wrapUp).toBe(false);
    }
  });

  test("blocks Steer on queued Antigravity rows even if a legacy Controller advertises steer=true", () => {
    const state = resolveDeliveryPickerState({
      busy: true,
      paused: false,
      queued: true,
      steerReady: false,
      wrapUpReady: false,
      contextReady: true,
      disallowSteer: true,
      fresh: true,
      error: null,
      options: {
        ...baseOptions,
        steer: true,
        controls: { version: 1, next_checkpoint: true },
        wrap_up_steering: false,
      },
    });
    expect(state.steer).toBe(false);
    expect(state.checkpoint).toBe(false);
    expect(state.showWrapUp).toBe(false);
    expect(state.unavailable).toBe("Not supported by this worker");
  });

  test("wrapUpQueueAddOptions preserves exact target and downgrades targetless submits to plain front queue", () => {
    expect(wrapUpQueueAddOptions("s1", { worker_epoch: "w1", turn_id: "t1" })).toEqual({
      mode: "front",
      origin: "composer",
      wrapUp: true,
      wrapUpSessionId: "s1",
      wrapUpTarget: { worker_epoch: "w1", turn_id: "t1" },
    });
    expect(wrapUpQueueAddOptions("s1", undefined)).toEqual({
      mode: "front",
      origin: "composer",
    });
  });

  test("enables Wrap up & send next on Owner-authored queued rows when Controller advertises queued support", () => {
    const owner = resolveDeliveryPickerState({
      busy: true,
      paused: false,
      queued: true,
      steerReady: false,
      wrapUpReady: true,
      contextReady: true,
      ownerAuthored: true,
      disallowSteer: true,
      fresh: true,
      error: null,
      options: baseOptions,
    });
    expect(owner.showWrapUp).toBe(true);
    expect(owner.wrapUp).toBe(true);
    expect(owner.steer).toBe(false);
    expect(owner.preferred).toBe("wrap_up");

    // Non-owner (Peer/Automation/legacy) queued rows stay visible as Wrap up but disabled with clear reason.
    const nonOwner = resolveDeliveryPickerState({
      busy: true,
      paused: false,
      queued: true,
      steerReady: false,
      wrapUpReady: true,
      contextReady: true,
      ownerAuthored: false,
      disallowSteer: true,
      fresh: true,
      error: null,
      options: baseOptions,
    });
    expect(nonOwner.showWrapUp).toBe(true);
    expect(nonOwner.wrapUp).toBe(false);
    expect(nonOwner.wrapUpUnavailable).toBe("Only your own messages can request wrap-up");

    // Older Controller without wrap_up_queued_supported falls back cleanly.
    const legacy = resolveDeliveryPickerState({
      busy: true,
      paused: false,
      queued: true,
      steerReady: false,
      wrapUpReady: true,
      contextReady: true,
      ownerAuthored: true,
      disallowSteer: true,
      fresh: true,
      error: null,
      options: { ...baseOptions, wrap_up_queued_supported: false },
    });
    expect(legacy.showWrapUp).toBe(false);
    expect(legacy.wrapUp).toBe(false);

    // Self-editing this queued row does not block wrap-up; editing another row still blocks it.
    const selfEditing = resolveDeliveryPickerState({
      busy: true,
      paused: false,
      queued: true,
      steerReady: false,
      wrapUpReady: true,
      contextReady: true,
      ownerAuthored: true,
      editingRowId: "q1",
      editingToken: "tok-q1",
      disallowSteer: true,
      fresh: true,
      error: null,
      options: { ...baseOptions, editing: true, editing_id: "q1", editing_token: "tok-q1" },
    });
    expect(selfEditing.wrapUp).toBe(true);

    const otherEditing = resolveDeliveryPickerState({
      busy: true,
      paused: false,
      queued: true,
      steerReady: false,
      wrapUpReady: true,
      contextReady: true,
      ownerAuthored: true,
      editingRowId: "q1",
      disallowSteer: true,
      fresh: true,
      error: null,
      options: { ...baseOptions, editing: true, editing_id: "q2" },
    });
    expect(otherEditing.wrapUp).toBe(false);
    expect(otherEditing.wrapUpUnavailable).toBe("Finish editing queued message first");
  });

  test("isOwnerAuthoredRow enforces C-auth provenance on confirmed server rows", () => {
    expect(isOwnerAuthoredRow(null)).toBe(true);
    expect(isOwnerAuthoredRow({ serverOrigin: { kind: "owner" } })).toBe(true);
    expect(isOwnerAuthoredRow({ serverOrigin: { kind: "peer", from_session: "s2" } })).toBe(false);
    expect(isOwnerAuthoredRow({ serverOrigin: { kind: "automation" } })).toBe(false);
    expect(isOwnerAuthoredRow({ serverOrigin: { kind: "cowboy", source: "schedule" } })).toBe(false);
    // Pre-C-auth legacy server row (no serverOrigin, no local status) -> non-owner.
    expect(isOwnerAuthoredRow({})).toBe(false);
    // Local optimistic browser row not yet echoed by server -> owner.
    expect(isOwnerAuthoredRow({ status: "sending" })).toBe(true);
  });

  test("wrapUpActivateFields attaches wrap-up fields only for front Owner activations on capable Controllers", () => {
    const target = { worker_epoch: "w1", turn_id: "t1" };
    expect(
      wrapUpActivateFields([{ id: "s1", wrap_up_steering: true }], "s1", {
        front: true,
        wrapUp: true,
        wrapUpSessionId: "s1",
        wrapUpTarget: target,
      }),
    ).toEqual({ wrap_up: true, wrap_up_target: target });
    expect(
      wrapUpActivateFields([{ id: "s1" }], "s1", {
        front: true,
        wrapUp: true,
        wrapUpSessionId: "s1",
        wrapUpTarget: target,
      }),
    ).toEqual({});
  });

  test("keeps native Steer intact for Codex/Claude sessions", () => {
    const state = resolveDeliveryPickerState({
      busy: true,
      paused: false,
      queued: false,
      steerReady: true,
      wrapUpReady: false,
      contextReady: true,
      fresh: true,
      error: null,
      options: { ...baseOptions, steer: true, wrap_up_steering: false },
    });
    expect(state.showWrapUp).toBe(false);
    expect(state.steer).toBe(true);
    expect(state.preferred).toBe("steer");
  });

  test("falls back to standard disabled Steer when wrap_up_steering is false or absent", () => {
    const state = resolveDeliveryPickerState({
      busy: true,
      paused: false,
      queued: false,
      steerReady: false,
      wrapUpReady: false,
      contextReady: true,
      fresh: true,
      error: null,
      options: { ...baseOptions, wrap_up_steering: false },
    });
    expect(state.showWrapUp).toBe(false);
    expect(state.wrapUp).toBe(false);
    expect(state.preferred).toBe("steer");
  });
});

describe("wrapUpSubmitFields (rollback-safe outbox wire encoding)", () => {
  const target = { worker_epoch: "w1", turn_id: "t1" };

  test("attaches wrap_up and wrap_up_target only when session matches and wrap_up_steering is true", () => {
    expect(
      wrapUpSubmitFields(
        [{ id: "s1", wrap_up_steering: true }],
        "s1",
        "frontQueue",
        { wrapUp: true, wrapUpSessionId: "s1", wrapUpTarget: target },
      ),
    ).toEqual({ wrap_up: true, wrap_up_target: target });
  });

  test("downgrades cleanly to plain front submit on legacy Controllers, mismatched session, or missing target", () => {
    expect(
      wrapUpSubmitFields(
        [{ id: "s1" }],
        "s1",
        "frontQueue",
        { wrapUp: true, wrapUpSessionId: "s1", wrapUpTarget: target },
      ),
    ).toEqual({});
    expect(
      wrapUpSubmitFields(
        [{ id: "s1", wrap_up_steering: false }],
        "s1",
        "frontQueue",
        { wrapUp: true, wrapUpSessionId: "s1", wrapUpTarget: target },
      ),
    ).toEqual({});
    expect(
      wrapUpSubmitFields(
        [{ id: "s1", wrap_up_steering: true }],
        "s1",
        "frontQueue",
        { wrapUp: true, wrapUpSessionId: "s2", wrapUpTarget: target },
      ),
    ).toEqual({});
    expect(
      wrapUpSubmitFields(
        [{ id: "s1", wrap_up_steering: true }],
        "s1",
        "frontQueue",
        { wrapUp: true, wrapUpSessionId: "s1" },
      ),
    ).toEqual({});
    expect(
      wrapUpSubmitFields(
        [{ id: "s1", wrap_up_steering: true }],
        "s1",
        "addQueue",
        { wrapUp: true, wrapUpSessionId: "s1", wrapUpTarget: target },
      ),
    ).toEqual({});
  });
});

describe("isCurrentContextReady", () => {
  const baseContext: SessionContext = {
    revision: 2,
    applied_revision: 2,
    sources: [],
    next_agent: null,
    source_sessions: [],
    delivery: null,
    max_sources: 5,
  };

  test("false when revision diverges or delivery is preparing/unknown", () => {
    expect(isCurrentContextReady(undefined)).toBe(true);
    expect(isCurrentContextReady(baseContext)).toBe(true);
    expect(isCurrentContextReady({ ...baseContext, revision: 3 })).toBe(false);
    expect(
      isCurrentContextReady({
        ...baseContext,
        delivery: { operation: "op1", session_id: "s1", state: "preparing" },
      }),
    ).toBe(false);
    expect(
      isCurrentContextReady({
        ...baseContext,
        delivery: { operation: "op1", session_id: "s1", state: "unknown" },
      }),
    ).toBe(false);
  });
});

describe("resolveDeliveryPickerState reopen cache isolation (R3-L1)", () => {
  test("ignores stale cached paused and editing flags on reopen until fresh read", () => {
    const staleCachedOptions = {
      target: { worker_epoch: "e1", turn_id: "t1" },
      controls: { next_checkpoint: false },
      steer: false,
      wrap_up_steering: true,
      editing: true,
      status: "Busy",
      paused: true,
      hold: null,
      policy: { mode: "steer" as const, updated_at_ms: 1 },
    };
    const reopened = resolveDeliveryPickerState({
      busy: true,
      paused: false,
      queued: false,
      steerReady: false,
      wrapUpReady: true,
      contextReady: true,
      fresh: false,
      error: null,
      options: staleCachedOptions,
    });
    expect(reopened.showWrapUp).toBe(true);
    expect(reopened.wrapUp).toBe(false);
    expect(reopened.wrapUpUnavailable).toBe("Checking runtime…");

    const refreshedPaused = resolveDeliveryPickerState({
      busy: true,
      paused: false,
      queued: false,
      steerReady: false,
      wrapUpReady: true,
      contextReady: true,
      fresh: true,
      error: null,
      options: { ...staleCachedOptions, editing: false, paused: true },
    });
    expect(refreshedPaused.wrapUp).toBe(false);
    expect(refreshedPaused.wrapUpUnavailable).toBe("Queue is paused");
  });
});

describe("bulkActivatableDrafts", () => {
  test("excludes recovery-* drafts while keeping ordinary drafts", () => {
    expect(isRecoveryDraftId("recovery-q1")).toBe(true);
    expect(isRecoveryDraftId("q1")).toBe(false);
    const filtered = bulkActivatableDrafts([
      { id: "q1", text: "normal" },
      { id: "recovery-q2", text: "ambiguous" },
      { id: "q3", text: "normal-2" },
    ]);
    expect(filtered.map((d) => d.id)).toEqual(["q1", "q3"]);
  });
});

test("queued row already at index 0 prefers wrap_up under next_turn policy, while native Steer keeps next_turn", () => {
  const state = resolveDeliveryPickerState({
    busy: true,
    paused: false,
    queued: true,
    queueCount: 0,
    steerReady: false,
    wrapUpReady: true,
    contextReady: true,
    ownerAuthored: true,
    disallowSteer: true,
    fresh: true,
    error: null,
    options: {
      ...baseOptions,
      policy: { mode: "next_turn" as const, revision: 1 },
    },
  });
  expect(state.showWrapUp).toBe(true);
  expect(state.wrapUp).toBe(true);
  expect(state.preferred).toBe("wrap_up");
});

test("same-row editing held by another tab (mismatched editing_token) still blocks wrap-up", () => {
  const stolen = resolveDeliveryPickerState({
    busy: true, paused: false, queued: true, steerReady: false, wrapUpReady: true,
    contextReady: true, ownerAuthored: true, editingRowId: "q-self", editingToken: "tok-tab-a",
    disallowSteer: true, fresh: true, error: null,
    options: { ...baseOptions, editing: true, editing_id: "q-self", editing_token: "tok-tab-b" },
  });
  expect(stolen.wrapUp).toBe(false);
  expect(stolen.wrapUpUnavailable).toBe("Finish editing queued message first");
});

test("staged draft wrap-up requires wrap_up_draft_supported for rollback safety", () => {
  const rollback = resolveDeliveryPickerState({
    busy: true, paused: false, queued: false, stagedDraft: true, steerReady: false, wrapUpReady: true,
    contextReady: true, ownerAuthored: true, disallowSteer: true, fresh: true, error: null,
    options: { ...baseOptions, wrap_up_draft_supported: false },
  });
  expect(rollback.showWrapUp).toBe(false);
  expect(rollback.wrapUp).toBe(false);
  expect(
    wrapUpActivateFields(
      [{ id: "s1", wrap_up_steering: true }], "s1",
      { front: true, wrapUp: true, wrapUpSessionId: "s1", wrapUpTarget: { worker_epoch: "w1", turn_id: "t1" }, editingToken: "tok-1" },
      false,
    ),
  ).toEqual({});
  expect(
    wrapUpActivateFields(
      [{ id: "s1", wrap_up_steering: true }], "s1",
      { front: true, wrapUp: true, wrapUpSessionId: "s1", wrapUpTarget: { worker_epoch: "w1", turn_id: "t1" }, editingToken: "tok-1" },
      true,
    ),
  ).toEqual({ wrap_up: true, wrap_up_target: { worker_epoch: "w1", turn_id: "t1" }, editing_token: "tok-1" });
});

test("native Codex/Claude queued head under next_turn policy does not default focus to Steer", () => {
  const native = resolveDeliveryPickerState({
    busy: true, paused: false, queued: true, queueCount: 0, steerReady: true, wrapUpReady: false,
    contextReady: true, fresh: true, error: null,
    options: { ...baseOptions, steer: true, wrap_up_steering: false, policy: { mode: "next_turn", revision: 1 } },
  });
  expect(native.steer).toBe(true);
  expect(native.preferred).toBe("front");
});
