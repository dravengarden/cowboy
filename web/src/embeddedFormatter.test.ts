import { test } from "bun:test";
import { assertEquals, assertStringIncludes } from "@std/assert";
import { formatEmbeddedSource } from "./embeddedFormatter.ts";

test("TypeScript embedded payload is parser-formatted", async () => {
  const formatted = await formatEmbeddedSource({
    language: "typescript",
    source: 'const ws=new WebSocket("ws://127.0.0.1");ws.addEventListener("open",()=>console.log("ready"))',
    columns: 54,
  });
  assertStringIncludes(formatted, "const ws = new WebSocket");
  assertStringIncludes(formatted, "\n");
  assertStringIncludes(formatted, "console.log(\"ready\")");
});

test("JavaScript syntax failure preserves decoded source", async () => {
  const source = "const broken = {";
  assertEquals(
    await formatEmbeddedSource({ language: "javascript", source, columns: 44 }),
    source,
  );
});

test("unsupported Python formatter preserves source for highlighting", async () => {
  const source = "for host in hosts: print(host)";
  assertEquals(
    await formatEmbeddedSource({ language: "python", source, columns: 44 }),
    source,
  );
});
