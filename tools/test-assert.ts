import {
  deepStrictEqual,
  notDeepStrictEqual,
  ok,
  strictEqual,
  notStrictEqual,
  match,
  fail,
} from "node:assert/strict";

export {
  ok as assert,
  deepStrictEqual as assertEquals,
  notDeepStrictEqual as assertNotEquals,
  strictEqual as assertStrictEquals,
  notStrictEqual as assertNotStrictEquals,
  match as assertMatch,
};

export function assertFalse(value: unknown, message?: string): void {
  strictEqual(value, false, message);
}

export function assertStringIncludes(actual: string, expected: string, message?: string): void {
  ok(
    actual.includes(expected),
    message ?? `Expected ${JSON.stringify(actual)} to include ${JSON.stringify(expected)}`,
  );
}

export function assertAlmostEquals(
  actual: number,
  expected: number,
  tolerance = 1e-7,
  message?: string,
): void {
  ok(
    Object.is(actual, expected) || Math.abs(actual - expected) <= tolerance,
    message ?? `Expected ${actual} within ${tolerance} of ${expected}`,
  );
}

type ErrorClass<E extends Error> = new (...args: any[]) => E;

function checkedError<E extends Error>(
  error: unknown,
  ErrorType?: ErrorClass<E>,
  includes?: string,
  message?: string,
): E {
  if (ErrorType) ok(error instanceof ErrorType, message ?? `Expected ${ErrorType.name}`);
  if (includes !== undefined) {
    ok(error !== null && typeof error === "object" && "message" in error);
    assertStringIncludes(String(error.message), includes, message);
  }
  return error as E;
}

/** Return the error so tests can inspect structured failure data. */
export function assertThrows<E extends Error = Error>(
  fn: () => unknown,
  ErrorType?: ErrorClass<E>,
  includes?: string,
  message?: string,
): E {
  try {
    fn();
  } catch (error) {
    return checkedError(error, ErrorType, includes, message);
  }
  fail(message ?? "Expected the function to throw");
}

export async function assertRejects<E extends Error = Error>(
  fn: () => PromiseLike<unknown>,
  ErrorType?: ErrorClass<E>,
  includes?: string,
  message?: string,
): Promise<E> {
  try {
    await fn();
  } catch (error) {
    return checkedError(error, ErrorType, includes, message);
  }
  fail(message ?? "Expected the promise to reject");
}
