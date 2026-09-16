// Where a link inside a rendered Markdown file points.
//
// A doc tree is a graph: `00-orientation.md` lists eleven siblings, and every
// one of those links used to leave the app (the renderer treats every href as
// external, so a relative path was resolved against the origin and fetched as a
// web page). Resolving them here turns the workspace's own documentation into
// something readable IN the reviewer, using the navigation stack that
// go-to-definition already uses.
//
// Pure and total: every href resolves to exactly one of these, including the
// hostile ones. The caller never has to guess.

export type ReviewLinkTarget =
  /** Leave the app: an absolute URL, a scheme we don't own, or a bare host. */
  | { kind: "external" }
  /** Same document: scroll to a heading. */
  | { kind: "anchor"; hash: string }
  /** Another file in this workspace. `line` comes from a GitHub-style `#L42`. */
  | { kind: "file"; path: string; line?: number; hash?: string }
  /** Resolvable, but not ours to open: it escapes the workspace root, or the
   *  href is empty. Callers must do nothing rather than guess a target. */
  | { kind: "unsupported" };

// Anything shaped like `scheme:` owns its own navigation — including `mailto:`,
// `tel:` and, importantly, `file:` (an absolute OS path is not a workspace
// path). A single leading `//` is protocol-relative, i.e. also a URL.
const SCHEME = /^[a-zA-Z][a-zA-Z0-9+.-]*:/;
const GITHUB_LINE = /^L(\d+)(?:-L?\d+)?$/;

function decode(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    // A malformed escape is not a reason to drop the link; the raw text is a
    // better guess at the author's intent than nothing.
    return value;
  }
}

/** POSIX-style resolution against the linking file's directory. Returns
 *  undefined when the path climbs out of the workspace root, which is the one
 *  case a viewer must refuse rather than clamp: silently opening the root's
 *  sibling would be a worse answer than not navigating. */
export function resolveWorkspacePath(
  fromPath: string,
  href: string,
): string | undefined {
  const segments = href.startsWith("/")
    ? href.slice(1).split("/")
    : [...fromPath.split("/").slice(0, -1), ...href.split("/")];
  const resolved: string[] = [];
  for (const segment of segments) {
    if (segment === "" || segment === ".") continue;
    if (segment === "..") {
      if (resolved.length === 0) return undefined;
      resolved.pop();
      continue;
    }
    resolved.push(segment);
  }
  return resolved.length > 0 ? resolved.join("/") : undefined;
}

export function resolveReviewLink(
  fromPath: string,
  href: string,
): ReviewLinkTarget {
  const raw = href.trim();
  if (raw === "") return { kind: "unsupported" };
  if (raw.startsWith("//") || SCHEME.test(raw)) return { kind: "external" };
  if (raw.startsWith("#")) {
    const hash = decode(raw.slice(1));
    return hash === "" ? { kind: "unsupported" } : { kind: "anchor", hash };
  }
  const hashAt = raw.indexOf("#");
  const hash = hashAt < 0 ? "" : decode(raw.slice(hashAt + 1));
  const withoutHash = hashAt < 0 ? raw : raw.slice(0, hashAt);
  // `?plain=1` and friends are viewer hints for the forge, not part of the path.
  const queryAt = withoutHash.indexOf("?");
  const pathPart = queryAt < 0 ? withoutHash : withoutHash.slice(0, queryAt);
  if (pathPart === "") {
    // `?query#hash` with no path is a self-reference to this document.
    return hash === "" ? { kind: "unsupported" } : { kind: "anchor", hash };
  }
  const path = resolveWorkspacePath(fromPath, decode(pathPart));
  if (path === undefined) return { kind: "unsupported" };
  const line = GITHUB_LINE.exec(hash);
  if (line?.[1]) {
    const parsed = Number.parseInt(line[1], 10);
    if (Number.isSafeInteger(parsed) && parsed > 0) {
      return { kind: "file", path, line: parsed };
    }
  }
  return hash === "" ? { kind: "file", path } : { kind: "file", path, hash };
}

/** GitHub-compatible-enough heading slug: lowercase, drop punctuation, spaces
 *  to dashes. Kept next to the resolver because the two must agree — a `#hash`
 *  produced by one is looked up by the other. Letters are matched by Unicode
 *  property, so a Chinese heading keeps its characters instead of slugging to
 *  the empty string. */
export function headingSlug(text: string): string {
  return text
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s-]/gu, "")
    .replace(/\s+/g, "-");
}
