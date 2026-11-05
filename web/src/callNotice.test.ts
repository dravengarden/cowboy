import { expect, test } from "bun:test";
import { callNoticeStateLabel, callNoticeTone, isCallNoticeCmid, parseCallNotice } from "./callNotice";

const done =
  "[Cowboy] Managed call call-75a40bd32bda4b984d2d6809 (Stormbird code-health plan, round 2 (after Astra Max findings)) finished: completed. Read it with cowboy_read: cowboy.calls.result(\"call-75a40bd32bda4b984d2d6809\",{offset:0,chars:4000}). A running, failed or partial review is never evidence of a clean review.";

test("a titled notice keeps nested parentheses in its title", () => {
  const notice = parseCallNotice(done);
  expect(notice).toEqual({
    callId: "call-75a40bd32bda4b984d2d6809",
    title: "Stormbird code-health plan, round 2 (after Astra Max findings)",
    state: "completed",
    error: null,
  });
  expect(notice && callNoticeTone(notice)).toBe("success");
  expect(notice && callNoticeStateLabel(notice)).toBe("Done");
});

test("untitled and failed notices carry the error code", () => {
  const notice = parseCallNotice(
    "[Cowboy] Managed call call-1 finished: failed, error provider_unavailable. Read it with cowboy_read: x",
  );
  expect(notice).toEqual({ callId: "call-1", title: null, state: "failed", error: "provider_unavailable" });
  expect(notice && callNoticeTone(notice)).toBe("error");
  expect(notice && callNoticeStateLabel(notice)).toBe("Failed · provider unavailable");
  const cancelled = parseCallNotice("[Cowboy] Managed call call-2 finished: cancelled.");
  expect(cancelled && callNoticeTone(cancelled)).toBe("warning");
});

test("other text and cmids are not call notices", () => {
  expect(parseCallNotice("Managed call call-1 finished: completed.")).toBeNull();
  expect(parseCallNotice("please check [Cowboy] Managed call call-1 finished: completed.")).toBeNull();
  expect(isCallNoticeCmid("call-notice:call-1")).toBe(true);
  expect(isCallNoticeCmid("q-123")).toBe(false);
  expect(isCallNoticeCmid(undefined)).toBe(false);
});
