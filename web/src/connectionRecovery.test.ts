import { assertEquals } from "jsr:@std/assert";
import {
  ForegroundProbe,
  isAppleTouchWebView,
  shouldReconnectOnForeground,
  shouldStartImmediateReconnect,
} from "./connectionRecovery.ts";

Deno.test("foreground recovery preserves an in-flight replacement", () => {
  assertEquals(shouldReconnectOnForeground(undefined, 0, 30_000), true);
  assertEquals(shouldReconnectOnForeground(0, 0, 30_000), false);
  assertEquals(shouldReconnectOnForeground(2, 0, 30_000), true);
  assertEquals(shouldReconnectOnForeground(3, 0, 30_000), true);
  assertEquals(shouldStartImmediateReconnect(0), false);
  assertEquals(shouldStartImmediateReconnect(1), true);
});

Deno.test("foreground recovery preserves a fresh open socket", () => {
  assertEquals(shouldReconnectOnForeground(1, 29_999, 30_000), false);
  assertEquals(shouldReconnectOnForeground(1, 30_001, 30_000), true);
});

Deno.test("Apple touch foreground recovery checks a ready socket and preserves bootstrap", () => {
  assertEquals(shouldReconnectOnForeground(1, 0, 30_000, true), true);
  assertEquals(shouldReconnectOnForeground(0, 0, 30_000, true), false);
  assertEquals(shouldReconnectOnForeground(1, 0, 30_000, true, false), false);
  assertEquals(shouldStartImmediateReconnect(1, false), false);
  // A live capacity queue keeps its socket; a dead queue must still recover.
  assertEquals(shouldStartImmediateReconnect(1, false, false, true), true);
  assertEquals(shouldStartImmediateReconnect(0, false, false, true), false);
  assertEquals(shouldStartImmediateReconnect(undefined, false, true), false);
  assertEquals(shouldReconnectOnForeground(undefined, 60_000, 30_000, true, false, true), false);
});

function probeHarness(): {
  probe: ForegroundProbe;
  expire: () => void;
  pending: () => number;
} {
  let id = 0;
  const callbacks = new Map<number, () => void>();
  const probe = new ForegroundProbe(4000, (callback, delay) => {
    assertEquals(delay, 4000);
    const timer = ++id;
    callbacks.set(timer, callback);
    return () => { callbacks.delete(timer); };
  });
  return {
    probe,
    expire: () => {
      const jobs = [...callbacks.values()];
      callbacks.clear();
      for (const job of jobs) job();
    },
    pending: () => callbacks.size,
  };
}

Deno.test("foreground probe coalesces repeated triggers and requires its own reply", () => {
  const { probe, expire, pending } = probeHarness();
  const sent: number[] = [];
  let retries = 0;
  const start = (): boolean => probe.start((nonce) => sent.push(nonce), () => retries++);
  assertEquals(start(), true);
  assertEquals(start(), false);
  assertEquals(sent.length, 1);
  assertEquals(probe.acknowledge(sent[0]! + 1), false);
  assertEquals(pending(), 1);
  assertEquals(probe.acknowledge(sent[0]!), true);
  expire();
  assertEquals(retries, 0);
  assertEquals(start(), true);
  assertEquals(probe.acknowledge(sent[0]!), false);
  expire();
  assertEquals(retries, 1);
  expire();
  assertEquals(retries, 1);
});

Deno.test("closing or hiding cancels a foreground probe; send failure recovers immediately", () => {
  const { probe, expire, pending } = probeHarness();
  let retries = 0;
  probe.start(() => {}, () => retries++);
  probe.cancel();
  expire();
  assertEquals(retries, 0);
  probe.start(() => { throw new Error("socket closed"); }, () => retries++);
  assertEquals(retries, 1);
  assertEquals(pending(), 0);
});

Deno.test("Apple touch WebView detection covers iPhone and desktop-UA iPad", () => {
  assertEquals(
    isAppleTouchWebView(
      "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)",
      "iPhone",
      5,
    ),
    true,
  );
  assertEquals(
    isAppleTouchWebView(
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
      "MacIntel",
      5,
    ),
    true,
  );
  assertEquals(
    isAppleTouchWebView(
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
      "MacIntel",
      0,
    ),
    false,
  );
  assertEquals(
    isAppleTouchWebView(
      "Mozilla/5.0 (Linux; Android 16)",
      "Linux armv8l",
      5,
    ),
    false,
  );
});
