import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  classifyNetwork,
  NO_ESTIMATE,
  type ThroughputEstimate,
  withThroughputSample,
} from "./bandwidth";

/** `count` downloads of `bytes` each taking `ms`. */
function samples(
  from: ThroughputEstimate,
  count: number,
  bytes: number,
  ms: number,
): ThroughputEstimate {
  let estimate = from;
  for (let index = 0; index < count; index++) {
    estimate = withThroughputSample(estimate, bytes, ms);
  }
  return estimate;
}

test("small or instant transfers say nothing about the link", () => {
  assertEquals(withThroughputSample(NO_ESTIMATE, 2048, 500), NO_ESTIMATE);
  assertEquals(withThroughputSample(NO_ESTIMATE, 64 * 1024, 0), NO_ESTIMATE);
});

test("a slow link becomes constrained only after a few samples", () => {
  // 64 KiB in 1 s is about 0.5 Mbit/s.
  const one = samples(NO_ESTIMATE, 1, 64 * 1024, 1000);
  assertEquals(one.constrained, false);
  const three = samples(NO_ESTIMATE, 3, 64 * 1024, 1000);
  assertEquals(three.constrained, true);
});

test("recovery needs a clear margin, so the condition does not flap", () => {
  const slow = samples(NO_ESTIMATE, 4, 64 * 1024, 1000);
  // One response at about 1.5 Mbit/s keeps it constrained.
  const marginal = withThroughputSample(slow, 192 * 1024, 1000);
  assertEquals(marginal.constrained, true);
  // A run of fast responses (about 50 Mbit/s) clears it.
  assertEquals(samples(slow, 6, 1024 * 1024, 160).constrained, false);
});

test("the browser's own hint wins when it has one", () => {
  const fast = samples(NO_ESTIMATE, 4, 1024 * 1024, 100);
  assertEquals(classifyNetwork(false, undefined, fast), "offline");
  assertEquals(
    classifyNetwork(true, { saveData: true }, fast),
    "constrained",
  );
  assertEquals(
    classifyNetwork(true, { effectiveType: "3g" }, fast),
    "constrained",
  );
  assertEquals(classifyNetwork(true, { downlink: 0.4 }, fast), "constrained");
  assertEquals(
    classifyNetwork(true, { effectiveType: "4g", downlink: 10 }, fast),
    "unconstrained",
  );
  // No hint (iOS WebKit): the passive estimate decides.
  const slow = samples(NO_ESTIMATE, 4, 64 * 1024, 1000);
  assertEquals(classifyNetwork(true, undefined, slow), "constrained");
  assertEquals(classifyNetwork(true, undefined, NO_ESTIMATE), "unconstrained");
});
