/** Closed Service observations. IDs, display paths and positions are not grants. */
import {
  type CapturedContent,
  type ContentIdentity,
  contentRequest,
} from "./content.ts";
import {
  decodeResourceId,
  integer,
  list,
  type Point,
  record,
  requireValue,
  type ResourceId,
  text,
} from "./protocol.ts";

declare const navigationId: unique symbol;
export type NavigationId = string & { readonly [navigationId]: true };
export type NavigationKind =
  | "definition"
  | "declaration"
  | "typeDefinition"
  | "implementation"
  | "references";
export type NavigationState =
  | "prepared"
  | "unknown"
  | "retained"
  | "release_unknown"
  | "released"
  | "expired";
export interface NavigationRequest {
  readonly content: ContentIdentity;
  readonly position: Point;
  readonly query: NavigationKind;
}
export interface NavigationLocation {
  readonly path: string;
  readonly content: ContentIdentity;
  readonly start: Point;
  readonly end: Point;
}
export interface NavigationSnapshot extends NavigationRequest {
  readonly apiVersion: 1;
  readonly navigationId: NavigationId;
  readonly sourceResourceId: ResourceId;
  readonly state: NavigationState;
  readonly locations: readonly NavigationLocation[];
  /** Destination handoff is a separate client continuation, not an ID importer. */
  readonly destinations: readonly never[];
  readonly pending: boolean;
}

export function navigationRequest(
  snapshot: CapturedContent,
  position: Point,
  query: NavigationKind,
): NavigationRequest {
  requireValue(
    query === "definition" || query === "declaration" ||
      query === "typeDefinition" ||
      query === "implementation" || query === "references",
  );
  // Reuse the actual captured-LF/UTF-16 boundary check, not an unbounded point.
  const request = contentRequest(snapshot, { kind: "hover", position });
  return Object.freeze({
    content: request.content,
    position: request.query.position,
    query,
  });
}

function identity(value: unknown): ContentIdentity {
  const row = record(value, ["sha256", "utf8Bytes"]);
  const sha256 = text(row.sha256, 64);
  requireValue(/^[0-9a-f]{64}$/.test(sha256));
  return Object.freeze({
    sha256,
    utf8Bytes: integer(row.utf8Bytes, 0, 4 * 1024 * 1024),
  });
}
function point(value: unknown, limit: number): Point {
  const row = record(value, ["row", "column"]);
  return Object.freeze({
    row: integer(row.row, 0, limit),
    column: integer(row.column, 0, limit),
  });
}
function sameContent(a: ContentIdentity, b: ContentIdentity) {
  return a.sha256 === b.sha256 && a.utf8Bytes === b.utf8Bytes;
}

export function decodeNavigation(
  value: unknown,
  status: number,
  source: ResourceId,
  expected: NavigationRequest,
  id?: NavigationId,
): NavigationSnapshot {
  const row = record(value, [
    "apiVersion",
    "navigationId",
    "sourceResourceId",
    "content",
    "position",
    "query",
    "state",
    "locations",
    "destinations",
    "pending",
  ]);
  requireValue(
    row.apiVersion === 1 && decodeResourceId(row.sourceResourceId) === source &&
      typeof row.navigationId === "string" &&
      /^nav-[0-9a-f]{32}-[0-9a-f]{16}$/.test(row.navigationId) &&
      !row.navigationId.endsWith("-0000000000000000") &&
      (!id || row.navigationId === id) && row.query === expected.query,
  );
  const content = identity(row.content);
  const position = point(row.position, content.utf8Bytes);
  requireValue(
    sameContent(content, expected.content) &&
      position.row === expected.position.row &&
      position.column === expected.position.column,
  );
  requireValue(
    row.state === "prepared" || row.state === "unknown" ||
      row.state === "retained" ||
      row.state === "release_unknown" || row.state === "released" ||
      row.state === "expired",
  );
  requireValue(
    typeof row.pending === "boolean" &&
      (row.pending ? status === 202 : status === 200) &&
      !((row.state === "released" || row.state === "expired") && row.pending),
  );
  const paths = new Map<string, ContentIdentity>();
  const locations = Object.freeze(
    list(row.locations, 256).map((value) => {
      const location = record(value, ["path", "content", "start", "end"]);
      const path = text(location.path, 4096);
      requireValue(
        path.length > 0 && !/[\p{Cc}]/u.test(path) &&
          !/[\uD800-\uDFFF]/u.test(path) &&
          path.split("/").every((part) =>
            part !== "" && part !== "." && part !== ".."
          ),
      );
      const content = identity(location.content);
      const previous = paths.get(path);
      requireValue(!previous || sameContent(previous, content));
      paths.set(path, content);
      const start = point(location.start, content.utf8Bytes);
      const end = point(location.end, content.utf8Bytes);
      requireValue(
        start.row < end.row ||
          start.row === end.row && start.column <= end.column,
      );
      return Object.freeze({ path, content, start, end });
    }),
  );
  requireValue(paths.size <= 32);
  requireValue(
    !["prepared", "unknown", "expired"].includes(row.state) ||
      locations.length === 0,
  );
  // This finite client has never requested a destination; never adopt unsolicited IDs.
  list(row.destinations, 0);
  return Object.freeze({
    apiVersion: 1,
    navigationId: row.navigationId as NavigationId,
    sourceResourceId: source,
    content,
    position,
    query: expected.query,
    state: row.state,
    locations,
    destinations: Object.freeze([]),
    pending: row.pending,
  });
}
