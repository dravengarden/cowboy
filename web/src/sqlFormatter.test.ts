import { assertEquals, assertStringIncludes } from "jsr:@std/assert";
import { formatEmbeddedFrame } from "./shellFormatter.ts";

Deno.test("PostgreSQL frames uppercase structure while preserving data casing", async () => {
  const frame = await formatEmbeddedFrame({
    launcher: "psql -c",
    language: "sql",
    dialect: "postgresql",
    text: "select replace(encode(convert_to(payload->>'command', 'UTF8'), 'base64'), chr(10), '') from events where payload->>'sessionUpdate'='tool_call' and jsonb_typeof(payload->'update'->'rawInput'->'command')='string'",
  }, 46);

  assertStringIncludes(frame.text, "SELECT\n");
  assertStringIncludes(frame.text, "FROM\n  events");
  assertStringIncludes(frame.text, "WHERE");
  assertStringIncludes(frame.text, "payload ->> 'command'");
  assertStringIncludes(frame.text, "'sessionUpdate'");
  assertStringIncludes(frame.text, "jsonb_typeof");
  assertEquals(frame.language, "sql");
});

Deno.test("invalid SQL fails closed to the decoded payload", async () => {
  const source = "select '";
  const frame = await formatEmbeddedFrame({ launcher: "psql -c", language: "sql", text: source }, 46);
  assertEquals(frame.text, source);
});
