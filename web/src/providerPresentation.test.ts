import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  providerName,
  providerPresentation,
  providerSelectionName,
} from "./providerPresentation";

test("unknown Providers degrade without an identity table", () => {
  const unknown = providerPresentation("future-agent");
  assertEquals(unknown, {
    agent: "future-agent",
    modelProvider: "",
    detail: "Provider catalog unavailable",
  });
  assertEquals(providerName("future-agent"), "future-agent");
  assertEquals(providerSelectionName("future-agent"), "future-agent");
});

test("empty Provider identity has an accessible generic fallback", () => {
  assertEquals(providerName(""), "Agent");
});
