import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import {
  classifyNetwork,
  lowPriorityPoll,
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

test("a low-priority poll never starts while its previous run is in flight", async () => {
  const pause = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
  let runs = 0;
  let settle: () => void = () => undefined;
  const stop = lowPriorityPoll(() => {
    runs++;
    return new Promise<void>((resolve) => {
      settle = resolve;
    });
  }, 2);
  await pause(30);
  // Many periods passed, but the first run has not settled.
  assertEquals(runs, 1);
  settle();
  await pause(30);
  assertEquals(runs, 2);
  stop();
  settle();
  await pause(20);
  assertEquals(runs, 2);
});
