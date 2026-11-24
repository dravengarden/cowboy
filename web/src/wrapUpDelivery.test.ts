import { describe, expect, test } from "bun:test";
import { isCurrentContextReady, type SessionContext } from "./sessionContext";
import { bulkActivatableDrafts, isRecoveryDraftId, resolveDeliveryPickerState, wrapUpQueueAddOptions, wrapUpSubmitFields } from "./wrapUpDelivery";

const baseOptions = {
  target: { worker_epoch: "w1", turn_id: "t1" },
  steer: false,
  wrap_up_steering: true,
  editing: false,
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

  test("never offers Wrap up & send next on queued rows (prevents Peer/Automation promotion)", () => {
    const state = resolveDeliveryPickerState({
      busy: true,
      paused: false,
      queued: true,
      steerReady: false,
      wrapUpReady: true,
      contextReady: true,
      fresh: true,
      error: null,
      options: baseOptions,
    });
    expect(state.showWrapUp).toBe(false);
    expect(state.wrapUp).toBe(false);
    expect(state.steer).toBe(false);
    expect(state.preferred).toBe("steer");
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
