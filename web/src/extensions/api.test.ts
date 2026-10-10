import { test } from "bun:test";
import { assertEquals, assertThrows } from "@std/assert";
import { decodeExtensionResponse, resourceQuery } from "./api.ts";

const item = {
  id: "12",
  title: "A repository resource",
  url: "https://example.com/o/r/issues/12",
  body: "# Description",
  bodyTruncated: false,
  state: "open",
  updatedAt: null,
  metadata: [],
};
test("extension responses reject executable links, unbounded collections and unknown failures", () => {
  assertEquals(decodeExtensionResponse({ type: "detail", item }), {
    type: "detail",
    item,
  });
  for (
    const url of [
      "javascript:alert(1)",
      "file:///etc/passwd",
      "https://secret@example.com/path",
    ]
  ) {
    assertThrows(() =>
      decodeExtensionResponse({ type: "detail", item: { ...item, url } })
    );
  }
  assertThrows(() =>
    decodeExtensionResponse({
      type: "page",
      items: Array(51).fill(item),
      nextPage: 2,
    })
  );
  assertThrows(() =>
    decodeExtensionResponse({ type: "page", items: [], nextPage: -1 })
  );
  assertThrows(() =>
    decodeExtensionResponse({ type: "unavailable", code: "raw_cli_stderr" })
  );
});
test("resource requests carry exact installation identity and escaped selections", () => {
  const identity = {
    pluginId: "fixture",
    pluginVersion: "1.0.0",
    generationDigest: `sha256:${"1".repeat(64)}`,
  };
  const query = resourceQuery(
    identity,
    "origin&remote=other",
    "issues",
    "open",
    2,
    "12",
  );
  assertEquals(query.get("remote"), "origin&remote=other");
  assertEquals(query.getAll("remote").length, 1);
  assertEquals(query.get("generationDigest"), identity.generationDigest);
  assertEquals(query.get("item"), "12");
});
