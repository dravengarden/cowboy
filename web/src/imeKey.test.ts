import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  IME_COMPOSITION_END_HOLD_MS,
  imeOwnsEditable,
  isImeInputType,
  isImeKeyEvent,
  isImeProtectedInput,
} from "./imeKey.ts";

test("IME keyboard events include active and legacy WebKit composition", () => {
  assertEquals(
    isImeKeyEvent({ isComposing: true, key: "Enter", keyCode: 13 }),
    true,
  );
  assertEquals(
    isImeKeyEvent({ isComposing: false, key: "Enter", keyCode: 229 }),
    true,
  );
  assertEquals(
    isImeKeyEvent({ isComposing: false, key: "Process", keyCode: 0 }),
    true,
  );
  assertEquals(
    isImeKeyEvent({ isComposing: false, key: "Dead", keyCode: 0 }),
    true,
  );
  assertEquals(
    isImeKeyEvent({ isComposing: false, key: "Enter", keyCode: 13 }),
    false,
  );
});

test("IME beforeinput types are the candidate-confirm family", () => {
  assertEquals(isImeInputType("insertCompositionText"), true);
  assertEquals(isImeInputType("insertFromComposition"), true);
  assertEquals(isImeInputType("insertReplacementText"), true);
  assertEquals(isImeInputType("deleteCompositionText"), true);
  assertEquals(isImeInputType("deleteByComposition"), true);
  assertEquals(isImeInputType("insertText"), false);
  assertEquals(isImeInputType("insertLineBreak"), false);
  assertEquals(isImeInputType("deleteContentBackward"), false);
  assertEquals(isImeInputType(undefined), false);
});

test("iOS IME backspace stays protected while composing", () => {
  assertEquals(
    isImeProtectedInput({
      inputType: "deleteContentBackward",
      isComposing: true,
    }),
    true,
  );
  assertEquals(
    isImeProtectedInput(
      { inputType: "deleteContentBackward", isComposing: false },
      true,
    ),
    true,
  );
  assertEquals(
    isImeProtectedInput({
      inputType: "deleteContentBackward",
      isComposing: false,
    }),
    false,
  );
});

test("iOS Pinyin compositionend still owns the editable until start or hold", () => {
  assertEquals(IME_COMPOSITION_END_HOLD_MS, 50);
  assertEquals(imeOwnsEditable(true, 0, 1_000), true);
  assertEquals(imeOwnsEditable(false, 0, 1_000), false);
  assertEquals(imeOwnsEditable(false, 1_000, 1_000), true);
  assertEquals(imeOwnsEditable(false, 1_000, 1_049), true);
  assertEquals(imeOwnsEditable(false, 1_000, 1_050), false);
  assertEquals(imeOwnsEditable(true, 1_000, 2_000), true);
});
