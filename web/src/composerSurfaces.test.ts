import { expect, test } from "bun:test";
import type { SessionAction } from "./agentCommands";
import { type ComposerModal, composerModalReducer } from "./composerSurfaces.ts";

const none: ComposerModal = { kind: "none" };
const schedule: ComposerModal = { kind: "schedule", target: { id: undefined, initial: null } };
const move: ComposerModal = { kind: "move", sourceId: "draft-1" };
const context: ComposerModal = { kind: "context", action: { kind: "reset" } as SessionAction };
const settings: ComposerModal = { kind: "toolbarSettings" };

test("opening a surface replaces whatever was open", () => {
  for (const before of [none, schedule, move, context, settings]) {
    for (const next of [schedule, move, context, settings]) {
      expect(composerModalReducer(before, { type: "open", modal: next as Exclude<ComposerModal, { kind: "none" }> }))
        .toBe(next);
    }
  }
});

test("a close dismisses only the surface it names", () => {
  expect(composerModalReducer(schedule, { type: "close", kind: "schedule" })).toEqual(none);
  expect(composerModalReducer(move, { type: "close", kind: "move" })).toEqual(none);
  // The replaced schedule dialog's exit fires after the move picker opened.
  expect(composerModalReducer(move, { type: "close", kind: "schedule" })).toBe(move);
  expect(composerModalReducer(none, { type: "close", kind: "context" })).toBe(none);
});
