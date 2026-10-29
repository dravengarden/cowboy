import { expect, test } from "bun:test";
import { singleFlight } from "./usageRefresh";

test("concurrent global refreshes share one request", async () => {
  let calls = 0;
  let release: () => void = () => undefined;
  const refresh = singleFlight(() => {
    calls += 1;
    return new Promise<void>((resolve) => {
      release = resolve;
    });
  });
  const first = refresh();
  expect(refresh()).toBe(first);
  expect(calls).toBe(1);
  release();
  await first;
  const next = refresh();
  expect(calls).toBe(2);
  release();
  await next;
});
