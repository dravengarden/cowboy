import { expect, test } from "bun:test";
import type { SessionAction } from "./agentCommands";
import { type ComposerModal, type ComposerSurfaces, composerSurfacesReducer } from "./composerSurfaces.ts";

type OpenModal = Exclude<ComposerModal, { kind: "none" }>;
const anchor = {} as HTMLElement;
const none: ComposerModal = { kind: "none" };
const schedule: OpenModal = { kind: "schedule", target: { id: undefined, initial: null } };
const move: OpenModal = { kind: "move", sourceId: "draft-1" };
const context: OpenModal = { kind: "context", action: { kind: "reset" } as SessionAction };
const delivery: OpenModal = { kind: "delivery", anchor };
const clear: OpenModal = { kind: "clear", anchor };
const stop: OpenModal = { kind: "stop" };
const at = (modal: ComposerModal, fullscreen = false): ComposerSurfaces => ({ fullscreen, modal });

test("opening a surface replaces whatever was open and keeps full-screen", () => {
  for (const before of [none, schedule, move, context, delivery, clear]) {
    for (const next of [schedule, move, context, delivery, clear, stop]) {
      expect(composerSurfacesReducer(at(before, true), { type: "open", modal: next })).toEqual(at(next, true));
    }
  }
});

test("a close dismisses only the surface it names", () => {
  expect(composerSurfacesReducer(at(schedule), { type: "close", kind: "schedule" })).toEqual(at(none));
  // The replaced schedule dialog's exit fires after the move picker opened.
  const moving = at(move);
  expect(composerSurfacesReducer(moving, { type: "close", kind: "schedule" })).toBe(moving);
  // The picker's Stop replaces it with the stop confirmation; the picker's own
  // close arriving afterwards must not dismiss that confirmation.
  const stopping = composerSurfacesReducer(at(delivery, true), { type: "open", modal: stop });
  expect(composerSurfacesReducer(stopping, { type: "close", kind: "delivery" })).toBe(stopping);
});

test("full-screen is independent of the slot", () => {
  const picking = at(delivery);
  expect(composerSurfacesReducer(picking, { type: "fullscreen", open: true })).toEqual(at(delivery, true));
  const expanded = at(none, true);
  expect(composerSurfacesReducer(expanded, { type: "fullscreen", open: true })).toBe(expanded);
  expect(composerSurfacesReducer(expanded, { type: "fullscreen", open: false })).toEqual(at(none));
});

test("a context reset closes full-screen and the clear popper only", () => {
  expect(composerSurfacesReducer(at(clear, true), { type: "contextReset" })).toEqual(at(none));
  expect(composerSurfacesReducer(at(schedule, true), { type: "contextReset" })).toEqual(at(schedule));
});
