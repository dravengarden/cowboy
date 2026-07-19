import { assertEquals, assertStringIncludes } from "jsr:@std/assert";
import { formatEmbeddedFrame } from "./shellFormatter.ts";

Deno.test("complex jq frame is parser-validated and reflowed", async () => {
  const frame = await formatEmbeddedFrame({
    launcher: "jq",
    language: "jq",
    text: '.endpoints[] | select(.type == "tailscale" and (has("detour") | not) and .domain_resolver.server == "dns-direct") | .inbounds[] | select(.type == "tun")',
  }, 46);
  assertStringIncludes(frame.text, "\n");
  assertStringIncludes(frame.text, '"tailscale"');
  assertStringIncludes(frame.text, "\n| select");
});

Deno.test("jq reflow does not split operators inside strings", async () => {
  const frame = await formatEmbeddedFrame({
    launcher: "jq",
    language: "jq",
    text: '.items[] | select(.label == "one | two, and three") | {id, label}',
  }, 42);
  assertStringIncludes(frame.text, '"one | two, and three"');
});

Deno.test("invalid jq falls back to the decoded source", async () => {
  const source = ".items[] | select(";
  const frame = await formatEmbeddedFrame({ launcher: "jq", language: "jq", text: source }, 42);
  assertEquals(frame.text, source);
});
