import { assertEquals, assertStringIncludes } from "jsr:@std/assert";
import { formatEmbeddedFrame } from "./shellFormatter.ts";

Deno.test("PostgreSQL frames format decoded SQL and preserve source casing", async () => {
  const frame = await formatEmbeddedFrame({
    launcher: "psql -c",
    language: "sql",
    dialect: "postgresql",
    text: "select replace(encode(convert_to(payload->>'command', 'UTF8'), 'base64'), chr(10), '') from events where payload->>'sessionUpdate'='tool_call' and jsonb_typeof(payload->'update'->'rawInput'->'command')='string'",
  }, 46);

  assertStringIncludes(frame.text, "select\n");
  assertStringIncludes(frame.text, "from\n  events");
  assertStringIncludes(frame.text, "payload ->> 'command'");
  assertEquals(frame.language, "sql");
});

Deno.test("invalid SQL fails closed to the decoded payload", async () => {
  const source = "select '";
  const frame = await formatEmbeddedFrame({ launcher: "psql -c", language: "sql", text: source }, 46);
  assertEquals(frame.text, source);
});
