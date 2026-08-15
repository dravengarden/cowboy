import { assertEquals } from "jsr:@std/assert";
import {
  providerName,
  providerPresentation,
  providerSelectionName,
} from "./providerPresentation";

Deno.test("unknown Providers degrade without an identity table", () => {
  const unknown = providerPresentation("future-agent");
  assertEquals(unknown, {
    agent: "future-agent",
    modelProvider: "",
    detail: "Provider catalog unavailable",
  });
  assertEquals(providerName("future-agent"), "future-agent");
  assertEquals(providerSelectionName("future-agent"), "future-agent");
});

Deno.test("empty Provider identity has an accessible generic fallback", () => {
  assertEquals(providerName(""), "Agent");
});
