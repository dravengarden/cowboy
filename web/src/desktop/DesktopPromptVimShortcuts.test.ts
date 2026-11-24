import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { setActiveVimOwner, setVimMode } from "../vimModeStore.ts";
import {
  clearPromptChord,
  createPromptChordTracker,
  evaluatePromptCanvasKey,
} from "./DesktopPromptVimShortcuts.ts";

function fakeKey(
  key: string,
  code: string,
  opts: { shiftKey?: boolean; onSink?: boolean } = {},
): KeyboardEvent {
  const target = {
    matches: (sel: string) =>
      opts.onSink === true && sel === "[data-vim-command-sink]",
  };
  return {
    key,
    code,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: opts.shiftKey ?? false,
    isComposing: false,
    keyCode: 0,
    target,
  } as unknown as KeyboardEvent;
}

test("empty New Prompt ArrowUp recalls last pending message unless completion is open", () => {
  const tracker = createPromptChordTracker();
  const hit = evaluatePromptCanvasKey({
    event: fakeKey("ArrowUp", "ArrowUp"),
    tracker,
    isNewPrompt: true,
    isEmpty: true,
    hasCompletionOpen: false,
    canRecallLast: true,
    canCycleQueue: true,
  });
  assertEquals(hit, { consumed: true, action: { kind: "recall-last-pending" } });

  const blocked = evaluatePromptCanvasKey({
    event: fakeKey("ArrowUp", "ArrowUp"),
    tracker,
    isNewPrompt: true,
    isEmpty: true,
    hasCompletionOpen: true,
    canRecallLast: true,
    canCycleQueue: true,
  });
  assertEquals(blocked, { consumed: false });
});

test("Vim Normal [q/]q cycles send order and ZZ/ZQ saves or discards buffer", () => {
  setActiveVimOwner("default");
  setVimMode("normal");
  const tracker = createPromptChordTracker();

  const armPrev = evaluatePromptCanvasKey({
    event: fakeKey("[", "BracketLeft", { onSink: true }),
    tracker,
    isNewPrompt: true,
    isEmpty: false,
    hasCompletionOpen: false,
    canRecallLast: false,
    canCycleQueue: true,
  });
  assertEquals(armPrev, { consumed: true });
  const prev = evaluatePromptCanvasKey({
    event: fakeKey("q", "KeyQ", { onSink: true }),
    tracker,
    isNewPrompt: true,
    isEmpty: false,
    hasCompletionOpen: false,
    canRecallLast: false,
    canCycleQueue: true,
  });
  assertEquals(prev, { consumed: true, action: { kind: "cycle-send-order-prev" } });

  const armZ = evaluatePromptCanvasKey({
    event: fakeKey("Z", "KeyZ", { shiftKey: true, onSink: true }),
    tracker,
    isNewPrompt: false,
    isEmpty: false,
    hasCompletionOpen: false,
    canRecallLast: false,
    canCycleQueue: true,
  });
  assertEquals(armZ, { consumed: true });
  const zq = evaluatePromptCanvasKey({
    event: fakeKey("Q", "KeyQ", { shiftKey: true, onSink: true }),
    tracker,
    isNewPrompt: false,
    isEmpty: false,
    hasCompletionOpen: false,
    canRecallLast: false,
    canCycleQueue: true,
  });
  assertEquals(zq, { consumed: true, action: { kind: "discard-and-close" } });

  // Escape consumes an armed chord without letting the event close the buffer.
  const armBracket = evaluatePromptCanvasKey({
    event: fakeKey("[", "BracketLeft", { onSink: true }),
    tracker,
    isNewPrompt: false,
    isEmpty: false,
    hasCompletionOpen: false,
    canRecallLast: false,
    canCycleQueue: true,
  });
  assertEquals(armBracket, { consumed: true });
  const esc = evaluatePromptCanvasKey({
    event: fakeKey("Escape", "Escape", { onSink: true }),
    tracker,
    isNewPrompt: false,
    isEmpty: false,
    hasCompletionOpen: false,
    canRecallLast: false,
    canCycleQueue: true,
  });
  assertEquals(esc, { consumed: true });
  assertEquals(tracker.prefix, null);
  clearPromptChord(tracker);
});
