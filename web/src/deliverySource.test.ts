import { expect, test } from "bun:test";
import { deliverySourceMatches, guardDeliveryChoice } from "./deliverySource.ts";

const attachments: string[] = [];
const source = { session: "s1", text: "hello", attachments };

test("a delivery source matches only the same session, text and attachment list", () => {
  expect(deliverySourceMatches(source, { ...source })).toBe(true);
  expect(deliverySourceMatches(null, source)).toBe(false);
  expect(deliverySourceMatches(source, { ...source, session: "s2" })).toBe(false);
  expect(deliverySourceMatches(source, { ...source, text: "hello!" })).toBe(false);
  expect(deliverySourceMatches(source, { ...source, attachments: [] })).toBe(false);
});

test("a current choice dismisses the picker before running with its arguments", () => {
  const calls: string[] = [];
  const choose = guardDeliveryChoice({
    isCurrent: () => true,
    closeStale: () => calls.push("close"),
    dismiss: () => calls.push("dismiss"),
  }, (target: string, next: boolean) => calls.push(`run:${target}:${next}`));
  choose("turn-1", true);
  expect(calls).toEqual(["dismiss", "run:turn-1:true"]);
});

test("a stale choice only closes the picker", () => {
  const calls: string[] = [];
  const choose = guardDeliveryChoice({
    isCurrent: () => false,
    closeStale: () => calls.push("close"),
    dismiss: () => calls.push("dismiss"),
  }, () => calls.push("run"));
  choose();
  expect(calls).toEqual(["close"]);
});
