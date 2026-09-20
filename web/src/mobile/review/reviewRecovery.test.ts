import { assertEquals } from "jsr:@std/assert";
import { CodeApiError } from "./codeApi.ts";
import {
  isRecoverableReviewFailure,
  reviewRetryDelayMs,
} from "./reviewRecovery.ts";

Deno.test("retry backs off and then holds one steady cadence", () => {
  const delays = [0, 1, 2, 3, 4, 5, 12].map(reviewRetryDelayMs);
  assertEquals(delays, [800, 2_000, 5_000, 15_000, 30_000, 30_000, 30_000]);
  assertEquals(reviewRetryDelayMs(-3), 800);
});

Deno.test("a reconnecting Machine and an offline phone both recover", () => {
  assertEquals(isRecoverableReviewFailure(new CodeApiError(502)), true);
  assertEquals(isRecoverableReviewFailure(new CodeApiError(503)), true);
  assertEquals(isRecoverableReviewFailure(new TypeError("Failed to fetch")), true);
});

Deno.test("a durable answer and an aborted load stay put", () => {
  assertEquals(isRecoverableReviewFailure(new CodeApiError(404)), false);
  assertEquals(isRecoverableReviewFailure(new CodeApiError(403)), false);
  assertEquals(
    isRecoverableReviewFailure(new DOMException("aborted", "AbortError")),
    false,
  );
});
