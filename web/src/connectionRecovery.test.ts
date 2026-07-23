import { assertEquals } from "jsr:@std/assert";
import {
  isAppleTouchWebView,
  shouldReconnectOnForeground,
} from "./connectionRecovery.ts";

Deno.test("foreground recovery replaces every non-open socket", () => {
  assertEquals(shouldReconnectOnForeground(undefined, 0, 30_000), true);
  assertEquals(shouldReconnectOnForeground(0, 0, 30_000), true);
  assertEquals(shouldReconnectOnForeground(2, 0, 30_000), true);
  assertEquals(shouldReconnectOnForeground(3, 0, 30_000), true);
});

Deno.test("foreground recovery preserves a fresh open socket", () => {
  assertEquals(shouldReconnectOnForeground(1, 29_999, 30_000), false);
  assertEquals(shouldReconnectOnForeground(1, 30_001, 30_000), true);
});

Deno.test("Apple touch foreground recovery replaces even a fresh open socket", () => {
  assertEquals(shouldReconnectOnForeground(1, 0, 30_000, true), true);
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
