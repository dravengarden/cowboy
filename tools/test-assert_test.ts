import { test, expect } from "bun:test";
import { assertEquals, assertAlmostEquals, assertThrows, assertRejects } from "./test-assert.ts";

test("native equality checks nested values without coercing primitives", () => {
  assertEquals({ values: new Set([1, 2]) }, { values: new Set([2, 1]) });
  expect(() => assertEquals({ value: 1 }, { value: "1" })).toThrow();
});

test("throw assertions retain the original error and validate type and message", () => {
  const error = new TypeError("expected detail");
  expect(
    assertThrows(
      () => {
        throw error;
      },
      TypeError,
      "detail",
    ),
  ).toBe(error);
  expect(() => assertThrows(() => {}, Error)).toThrow();
  expect(() =>
    assertThrows(() => {
      throw error;
    }, RangeError),
  ).toThrow();
  expect(() =>
    assertThrows(
      () => {
        throw error;
      },
      TypeError,
      "missing",
    ),
  ).toThrow();
});

test("rejection assertions preserve errors and reject successful promises", async () => {
  const abort = new DOMException("aborted", "AbortError");
  expect(await assertRejects(() => Promise.reject(abort))).toBe(abort);
  const error = new AggregateError([], "failed");
  expect(await assertRejects(() => Promise.reject(error), AggregateError, "failed")).toBe(error);
  await expect(assertRejects(() => Promise.resolve())).rejects.toThrow();
});

test("numeric tolerance detects out-of-range and NaN values", () => {
  assertAlmostEquals(0.1 + 0.2, 0.3);
  expect(() => assertAlmostEquals(1, 2, 0.1)).toThrow();
  expect(() => assertAlmostEquals(Number.NaN, 2)).toThrow();
});
