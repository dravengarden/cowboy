import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  applyVisualHostPlugins,
  readableProviderAccent,
} from "./providerVisual.ts";
import { testFirstPartyHostPlugins } from "./testFirstPartyHostInventory.test.ts";

applyVisualHostPlugins(testFirstPartyHostPlugins());

test("Grok cream stays on dark paper and darkens on light paper", () => {
  assertEquals(readableProviderAccent("#E8E4DC", "dark", "#FFFFFF"), "#E8E4DC");
  assertEquals(readableProviderAccent("#E8E4DC", "light", "#1A1523"), "#44403C");
});

test("near-black Grok marks lift on dark paper and stay on light paper", () => {
  assertEquals(readableProviderAccent("#18181B", "dark", "#FFFFFF"), "#FFFFFF");
  assertEquals(readableProviderAccent("#18181B", "light", "#1A1523"), "#18181B");
});

test("already-readable Provider accents are left alone", () => {
  assertEquals(readableProviderAccent("#C65D3A", "light", "#1A1523"), "#C65D3A");
  assertEquals(readableProviderAccent("#E08A6A", "dark", "#FFFFFF"), "#E08A6A");
});
