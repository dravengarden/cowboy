import { test } from "bun:test";
import { assert, assertStringIncludes } from "@std/assert";
import { createElement } from "react";
import ReactMarkdown from "react-markdown";
import { renderToStaticMarkup } from "react-dom/server";
import { markdownRemarkPlugins } from "./markdownPlugins.ts";

function render(text: string, frontmatter: boolean): string {
  return renderToStaticMarkup(
    createElement(ReactMarkdown, {
      remarkPlugins: markdownRemarkPlugins(frontmatter),
    }, text),
  );
}

test("chat prose keeps a reply wrapped in --- rules", () => {
  const html = render(
    "---\n\nHi Zee, Chengjun needs to approve #8168 first.\n\n---",
    false,
  );
  assertStringIncludes(html, "Chengjun needs to approve");
  assertStringIncludes(html, "<hr/>");
});

test("a Markdown document hides its leading front matter", () => {
  const html = render("---\nname: skill\n---\n\n# Title", true);
  assertStringIncludes(html, "Title");
  assert(!html.includes("name: skill"));
});
