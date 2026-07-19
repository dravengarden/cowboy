import { assertEquals } from "jsr:@std/assert";
import { terminalDisplaySegments, terminalHighlightSegments } from "./terminalHighlight.ts";

Deno.test("whole JSON output is highlighted", () => {
  assertEquals(terminalHighlightSegments('{"ok":true}'), [{ text: '{"ok":true}', language: "json" }]);
});

Deno.test("plain prefix and JSON payload are split without changing bytes", () => {
  const text = 'wrote manifest\n{\n  "schema": 1,\n  "nested": {"ok": true}\n}\n';
  const segments = terminalHighlightSegments(text);
  assertEquals(segments.map((segment) => segment.language), [undefined, "json", undefined]);
  assertEquals(segments.map((segment) => segment.text).join(""), text);
});

Deno.test("braces in JSON strings do not end an island", () => {
  const text = 'result:\n{"message":"still } valid", "items":[1, 2]}';
  const segments = terminalHighlightSegments(text);
  assertEquals(segments[1]?.language, "json");
  assertEquals(segments.map((segment) => segment.text).join(""), text);
});

Deno.test("truncated JSON and ordinary logs stay plain", () => {
  assertEquals(terminalHighlightSegments('log\n{"partial": true'), [{ text: 'log\n{"partial": true' }]);
  assertEquals(terminalHighlightSegments("Build completed successfully."), [{ text: "Build completed successfully." }]);
});

Deno.test("diff and SQL output use deterministic languages", () => {
  assertEquals(terminalHighlightSegments("@@ -1 +1 @@\n-old\n+new")[0]?.language, "diff");
  assertEquals(terminalHighlightSegments("SELECT id FROM sessions;")[0]?.language, "sql");
});

Deno.test("display segments omit whitespace-only cards between JSON values", () => {
  const segments = terminalDisplaySegments('{"first":1}\n\n{"second":2}');
  assertEquals(segments.map((segment) => segment.language), ["json", "json"]);
  assertEquals(segments.some((segment) => segment.text.trim() === ""), false);
});
