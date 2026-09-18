import { assert, assertEquals, assertThrows } from "jsr:@std/assert";
import { BufferClientError, decodeResourceId } from "./protocol.ts";
import { decodeNavigation } from "./navigationProtocol.ts";
import { golden } from "./destinationFixture.ts";
import { ID, OTHER } from "./fixture.ts";
const expected = {
  content: golden.content,
  position: golden.position,
  query: "definition" as const,
};
const decode = (value: unknown, requested = new Set([0])) =>
  decodeNavigation(
    value,
    200,
    decodeResourceId(ID),
    expected,
    undefined,
    requested,
  );

Deno.test("actual Service destination wire is frozen, closed and accepted only for requested targets", () => {
  const value = decode(golden);
  assertEquals(value, golden);
  assert(
    Object.isFrozen(value.destinations) &&
      Object.isFrozen(value.destinations[0]),
  );
  assertThrows(() => decode(golden, new Set()), BufferClientError);
  for (const state of ["pending", "unknown", "expired"]) {
    assertEquals(
      decode({
        ...golden,
        destinations: [{ destination: 0, state, resourceId: null }],
      }).destinations[0]!.state,
      state,
    );
  }
});

Deno.test("destination observations reject extra fields, foreign/duplicate IDs, nonascending and unsolicited indices", () => {
  const entry = golden.destinations[0]!;
  const bad = [
    { ...entry, destination: -1 },
    { ...entry, destination: 0.5 },
    { ...entry, destination: 1 },
    { ...entry, resourceId: ID },
    { ...entry, resourceId: null },
    { ...entry, resourceId: golden.navigationId },
    { ...entry, state: "open" },
    { ...entry, state: "unknown" },
    { ...entry, state: "expired" },
    { ...entry, lease: "native" },
    { destination: 0, state: "prepared" },
  ];
  for (const item of bad) {
    assertThrows(
      () => decode({ ...golden, destinations: [item] }),
      BufferClientError,
    );
  }
  assertThrows(
    () => decode({ ...golden, destinations: [entry, entry] }),
    BufferClientError,
  );
  const locations = [golden.locations[0]!, {
    ...golden.locations[0]!,
    path: "other.rs",
  }];
  assertThrows(
    () =>
      decode({
        ...golden,
        locations,
        destinations: [entry, { ...entry, destination: 1 }],
      }, new Set([0, 1])),
    BufferClientError,
  );
  assertThrows(
    () =>
      decode({
        ...golden,
        locations,
        destinations: [{
          ...entry,
          destination: 1,
          resourceId: OTHER.slice(0, -1) + "3",
        }, entry],
      }, new Set([0, 1])),
    BufferClientError,
  );
});
