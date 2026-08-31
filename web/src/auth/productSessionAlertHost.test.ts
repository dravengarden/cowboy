import { assertEquals, assertStrictEquals } from "jsr:@std/assert";
import {
  productSessionAlertHost,
  setProductSessionAlertHost,
  subscribeProductSessionAlertHost,
} from "./productSessionAlertHost.ts";

Deno.test("desktop session alert host publishes only real mount changes", () => {
  const first = {} as HTMLElement;
  let updates = 0;
  const unsubscribe = subscribeProductSessionAlertHost(() => updates++);

  setProductSessionAlertHost(first);
  assertStrictEquals(productSessionAlertHost(), first);
  assertEquals(updates, 1);

  setProductSessionAlertHost(first);
  assertEquals(updates, 1);

  setProductSessionAlertHost(null);
  assertStrictEquals(productSessionAlertHost(), null);
  assertEquals(updates, 2);

  unsubscribe();
  setProductSessionAlertHost(first);
  assertEquals(updates, 2);
  setProductSessionAlertHost(null);
});
