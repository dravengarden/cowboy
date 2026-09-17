import { assert, assertEquals, assertThrows } from "jsr:@std/assert";
import { captureContent, type CapturedContent } from "./content.ts";
import { BufferClientError, decodeResourceId } from "./protocol.ts";
import {
  decodeNavigation,
  type NavigationKind,
  navigationRequest,
} from "./navigationProtocol.ts";
import { content, golden, navigationWire } from "./navigationFixture.ts";
import { OTHER } from "./fixture.ts";

const source = decodeResourceId(golden.sourceResourceId);
const request = {
  content: golden.content,
  position: golden.position,
  query: "definition" as const,
};
const decode = (value: unknown, status = 200) =>
  decodeNavigation(value, status, source, request);

Deno.test("navigation decodes the actual shared Service wire and freezes all evidence", () => {
  const value = decode(golden);
  assertEquals(value, golden);
  assert(
    Object.isFrozen(value) && Object.isFrozen(value.content) &&
      Object.isFrozen(value.position),
  );
  assert(
    Object.isFrozen(value.locations) && Object.isFrozen(value.destinations),
  );
  for (const location of value.locations) {
    assert(
      Object.isFrozen(location) && Object.isFrozen(location.start) &&
        Object.isFrozen(location.content),
    );
  }
  assertEquals(decode(navigationWire("prepared", true), 202).pending, true);
  for (
    const query of [
      "definition",
      "declaration",
      "typeDefinition",
      "implementation",
      "references",
    ] as const
  ) {
    assertEquals(
      decodeNavigation({ ...golden, query }, 200, source, { ...request, query })
        .query,
      query,
    );
  }
});

Deno.test("navigation requests require genuine complete LF capture and exact UTF-16 boundary", async () => {
  const captured = await content();
  assertEquals(
    navigationRequest(captured, golden.position, "definition"),
    request,
  );
  const unicode = await captureContent("a😀\nz");
  for (
    const position of [
      { row: 0, column: 2 },
      { row: 0, column: 4 },
      { row: 2, column: 0 },
      { row: -1, column: 0 },
      { row: 0, column: 0.5 },
    ]
  ) {
    assertThrows(
      () => navigationRequest(unicode, position, "definition"),
      BufferClientError,
    );
  }
  assertEquals(
    navigationRequest(unicode, { row: 0, column: 3 }, "references").position
      .column,
    3,
  );
  for (const fake of [{ text: "abc" }, structuredClone(captured)]) {
    assertThrows(
      () =>
        navigationRequest(
          fake as CapturedContent,
          golden.position,
          "definition",
        ),
      BufferClientError,
    );
  }
  assertThrows(
    () =>
      navigationRequest(
        captured,
        golden.position,
        "write_file" as NavigationKind,
      ),
    BufferClientError,
  );
});

Deno.test("navigation rejects foreign identity, open fields, invalid phases and unsolicited destination IDs", () => {
  for (
    const value of [
      { ...golden, privateNative: {} },
      { ...golden, apiVersion: 2 },
      { ...golden, sourceResourceId: OTHER },
      { ...golden, navigationId: OTHER },
      {
        ...golden,
        navigationId: "nav-" + "a".repeat(32) + "-0000000000000000",
      },
      { ...golden, content: { ...golden.content, sha256: "A".repeat(64) } },
      { ...golden, content: { ...golden.content, utf8Bytes: 4 } },
      { ...golden, content: { ...golden.content, path: "secret" } },
      { ...golden, position: { row: 0, column: 0 } },
      { ...golden, position: { ...golden.position, offset: 1 } },
      { ...golden, query: "references" },
      { ...golden, state: "pending" },
      { ...golden, pending: true },
      { ...golden, pending: "false" },
      {
        ...golden,
        destinations: [{
          destination: 0,
          resourceId: OTHER,
          state: "prepared",
        }],
      },
      ...["prepared", "unknown", "expired"].map((state) => ({
        ...golden,
        state,
      })),
    ]
  ) assertThrows(() => decode(value), BufferClientError, "protocol");
  assertThrows(() => decode(golden, 202), BufferClientError);
  assertThrows(
    () => decode(navigationWire("released", true), 202),
    BufferClientError,
  );
  assertThrows(
    () =>
      decodeNavigation(
        golden,
        200,
        source,
        request,
        decode({ ...golden, navigationId: "nav-" + OTHER }).navigationId,
      ),
    BufferClientError,
  );
});

Deno.test("navigation bounds and validates every target without interpreting display paths as ownership", () => {
  const location = golden.locations[0]!;
  const badLocations = [
    ...[
      "",
      "/absolute",
      "../escape",
      "a/./b",
      "a//b",
      "a/",
      "a\nsecret",
      "\uD800",
      "é".repeat(2049),
    ].map((path) => ({ ...location, path })),
    { ...location, start: { row: 0, column: 3 }, end: { row: 0, column: 2 } },
    { ...location, end: { row: 4, column: 0 } },
    {
      ...location,
      content: { ...location.content, utf8Bytes: 4 * 1024 * 1024 + 1 },
    },
    { ...location, content: { ...location.content, sha256: "xyz" } },
    { ...location, lease: "private" },
  ];
  for (const bad of badLocations) {
    assertThrows(
      () => decode({ ...golden, locations: [bad] }),
      BufferClientError,
    );
  }
  assertThrows(
    () => decode({ ...golden, locations: Array(257).fill(location) }),
    BufferClientError,
  );
  assertThrows(
    () =>
      decode({
        ...golden,
        locations: Array.from(
          { length: 33 },
          (_, n) => ({ ...location, path: `target${n}` }),
        ),
      }),
    BufferClientError,
  );
  assertThrows(
    () =>
      decode({
        ...golden,
        locations: [location, {
          ...location,
          content: { ...location.content, sha256: "b".repeat(64) },
        }],
      }),
    BufferClientError,
  );
  assertEquals(
    decode({ ...golden, locations: [{ ...location, path: "src/😀.rs" }] })
      .locations.length,
    1,
  );
});
