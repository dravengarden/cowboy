import { assert, assertEquals } from "jsr:@std/assert";
import {
  headingSlug,
  resolveReviewLink,
  resolveWorkspacePath,
} from "./reviewLinkTarget.ts";

const DOC = "docs/marketplace-service/zh/00-orientation.md";

Deno.test("a sibling link resolves inside the doc's own directory", () => {
  assertEquals(resolveReviewLink(DOC, "01-business-primer.md"), {
    kind: "file",
    path: "docs/marketplace-service/zh/01-business-primer.md",
  });
  assertEquals(resolveReviewLink(DOC, "./01-business-primer.md"), {
    kind: "file",
    path: "docs/marketplace-service/zh/01-business-primer.md",
  });
  assertEquals(resolveReviewLink(DOC, "../en/00-orientation.md"), {
    kind: "file",
    path: "docs/marketplace-service/en/00-orientation.md",
  });
  // Workspace-absolute, as a doc written against the repo root would say.
  assertEquals(resolveReviewLink(DOC, "/AGENTS.md"), {
    kind: "file",
    path: "AGENTS.md",
  });
});

Deno.test("anything that owns its own navigation stays external", () => {
  for (
    const href of [
      "https://doc.suger.io",
      "http://example.com/a.md",
      "mailto:a@b.c",
      "tel:+1",
      "//cdn.example.com/x.md",
      // An absolute OS path is NOT a workspace path.
      "file:///etc/passwd",
      "javascript:alert(1)",
    ]
  ) {
    assertEquals(resolveReviewLink(DOC, href), { kind: "external" }, href);
  }
});

Deno.test("a link out of the workspace is refused, not clamped", () => {
  // Climbing past the root would otherwise silently open a sibling of the
  // workspace — a worse answer than not navigating.
  assertEquals(resolveReviewLink("a/b.md", "../../../etc/passwd"), {
    kind: "unsupported",
  });
  assertEquals(resolveWorkspacePath("a/b.md", "../.."), undefined);
  assertEquals(resolveReviewLink(DOC, ""), { kind: "unsupported" });
  assertEquals(resolveReviewLink(DOC, "   "), { kind: "unsupported" });
  assertEquals(resolveReviewLink(DOC, "#"), { kind: "unsupported" });
});

Deno.test("fragments choose between an anchor, a line, and a path", () => {
  assertEquals(resolveReviewLink(DOC, "#只有-30-分钟的话"), {
    kind: "anchor",
    hash: "只有-30-分钟的话",
  });
  assertEquals(resolveReviewLink(DOC, "09-engineering-workflow.md#pr-gates"), {
    kind: "file",
    path: "docs/marketplace-service/zh/09-engineering-workflow.md",
    hash: "pr-gates",
  });
  // Climbing exactly to the root is legitimate reach, not an escape.
  assertEquals(resolveReviewLink(DOC, "../../../src/main.rs"), {
    kind: "file",
    path: "src/main.rs",
  });
  // GitHub line anchors become a reveal line, not a heading lookup.
  assertEquals(resolveReviewLink("a/b/c.md", "../d.rs#L42"), {
    kind: "file",
    path: "a/d.rs",
    line: 42,
  });
  assertEquals(resolveReviewLink("a/b/c.md", "../d.rs#L42-L50"), {
    kind: "file",
    path: "a/d.rs",
    line: 42,
  });
  assertEquals(resolveReviewLink("a/b/c.md", "d.rs#L0"), {
    kind: "file",
    path: "a/b/d.rs",
    hash: "L0",
  });
});

Deno.test("percent escapes and forge query hints are handled", () => {
  assertEquals(resolveReviewLink(DOC, "my%20notes.md"), {
    kind: "file",
    path: "docs/marketplace-service/zh/my notes.md",
  });
  assertEquals(resolveReviewLink(DOC, "01-business-primer.md?plain=1"), {
    kind: "file",
    path: "docs/marketplace-service/zh/01-business-primer.md",
  });
  assertEquals(resolveReviewLink(DOC, "?plain=1#top"), {
    kind: "anchor",
    hash: "top",
  });
  // A broken escape keeps the literal text rather than dropping the link.
  assertEquals(resolveReviewLink(DOC, "100%.md"), {
    kind: "file",
    path: "docs/marketplace-service/zh/100%.md",
  });
});

Deno.test("heading slugs survive punctuation and Chinese headings", () => {
  assertEquals(headingSlug("Reading order"), "reading-order");
  assertEquals(headingSlug("只有 30 分钟的话"), "只有-30-分钟的话");
  assertEquals(
    headingSlug("PR gates: what blocks a merge?"),
    "pr-gates-what-blocks-a-merge",
  );
  assertEquals(headingSlug("  Spaced  Out  "), "spaced-out");
});

const reviewApp = await Deno.readTextFile(
  new URL("./ReviewApp.tsx", import.meta.url),
);
const markdown = await Deno.readTextFile(
  new URL("../../Markdown.tsx", import.meta.url),
);
const markdownImpl = await Deno.readTextFile(
  new URL("../../MarkdownImpl.tsx", import.meta.url),
);

Deno.test("a doc link and a symbol jump share one history", () => {
  const follow = reviewApp.slice(
    reviewApp.indexOf("const followMarkdownLink"),
    reviewApp.indexOf("const navigateBack"),
  );
  // The SAME stack the code navigation arrows already read: a reader must not
  // have to remember which Back a given jump answers to.
  assert(follow.includes("setNavigationHistory((history) =>"));
  assert(follow.includes("setNavigationForwardHistory([])"));
  assert(follow.includes("openSource(resolved.path, resolved.line, true)"));
  // No second history was introduced for documents.
  assertEquals(reviewApp.includes("markdownHistory"), false);
  assertEquals(reviewApp.includes("docHistory"), false);
  // An external link keeps the existing opener; an unresolvable one is claimed
  // and reported rather than opened against this origin.
  assert(follow.includes('if (resolved.kind === "external") return false;'));
  assert(follow.includes("That link points outside this workspace."));
  assert(reviewApp.includes("onMarkdownLink={followMarkdownLink}"));
  assert(reviewApp.includes("previewAnchor={previewAnchor}"));
});

Deno.test("the renderer lets a host claim a link, and heads have slugs", () => {
  assert(markdown.includes("onLinkClick"));
  const anchor = markdownImpl.slice(
    markdownImpl.indexOf("a({ children, href })"),
  );
  assert(anchor.includes("onLinkClick?.(href, event) === true"));
  // Claimed links must not also reach the external opener.
  assert(
    anchor.indexOf("event.preventDefault();\n              return;") <
      anchor.indexOf("shouldRouteExternalClick"),
  );
  assert(
    markdownImpl.includes("const slug = headingSlug(headingText(children))"),
  );
  assert(markdownImpl.includes('h1: makeHeading("1.35em", 1.3, "h1")'));
});

Deno.test("the anchor scroll is measured, never scrollIntoView", () => {
  const effect = reviewApp.slice(
    reviewApp.indexOf("const appliedAnchor = useRef(0)"),
  );
  const block = effect.slice(0, effect.indexOf("\n  }, ["));
  // scrollIntoView on iOS may scroll an ancestor instead — inside the peek
  // compositor that drags the whole workspace sideways. (Comments stripped:
  // the code explains WHY it avoids the call, and would otherwise match.)
  const executable = block.replace(/^\s*\/\/.*$/gm, "");
  assertEquals(executable.includes("scrollIntoView"), false);
  assert(block.includes("getBoundingClientRect()"));
  assert(block.includes("appliedAnchor.current = previewAnchor.id"));
});
