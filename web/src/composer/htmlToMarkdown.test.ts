import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  htmlHasMarkdownStructure,
  type HtmlNodeLike,
  htmlNodeToMarkdown,
} from "./htmlToMarkdown.ts";

type Child = HtmlNodeLike | string;
function el(
  name: string,
  attrs: Record<string, string> = {},
  ...kids: Child[]
): HtmlNodeLike {
  const childNodes = kids.map((k) => (typeof k === "string" ? txt(k) : k));
  return {
    nodeType: 1,
    nodeName: name.toUpperCase(),
    childNodes,
    get textContent() {
      return childNodes.map((c) => c.textContent).join("");
    },
    getAttribute: (n) => (n in attrs ? attrs[n]! : null),
  };
}
function txt(value: string): HtmlNodeLike {
  return { nodeType: 3, nodeName: "#text", textContent: value, childNodes: [] };
}
const body = (...kids: Child[]) => el("body", {}, ...kids);

test("headings, emphasis, links and paragraphs", () => {
  assertEquals(
    htmlNodeToMarkdown(body(
      el("h2", {}, "Title"),
      el(
        "p",
        {},
        "Some ",
        el("strong", {}, "bold "),
        "and ",
        el("em", {}, "it"),
        " ",
        el("a", { href: "https://example.com/a b" }, "link"),
        ".",
      ),
      el("p", {}, "snake_case keeps ", el("code", {}, "x`y"), " [x]"),
    )),
    "## Title\n\nSome **bold** and *it* [link](https://example.com/a%20b).\n\nsnake_case keeps ``x`y`` \\[x\\]",
  );
});

test("nested lists, tasks and quotes", () => {
  assertEquals(
    htmlNodeToMarkdown(body(
      el(
        "ul",
        {},
        el(
          "li",
          {},
          "one",
          el("ol", { start: "3" }, el("li", {}, "three"), el("li", {}, "four")),
        ),
        el("li", {}, el("input", { type: "checkbox", checked: "" }), "done"),
      ),
      el("blockquote", {}, el("p", {}, "quoted"), el("p", {}, "twice")),
    )),
    "- one\n  3. three\n  4. four\n- [x] done\n\n> quoted\n>\n> twice",
  );
});

test("code blocks, tables, images and unsafe URLs", () => {
  assertEquals(
    htmlNodeToMarkdown(body(
      el("pre", {}, el("code", { class: "language-ts" }, "const a = 1;\n")),
      el(
        "table",
        {},
        el("tr", {}, el("th", {}, "A"), el("th", {}, "B|C")),
        el("tr", {}, el("td", {}, "1"), el("td", {}, "2")),
      ),
      el(
        "p",
        {},
        el("img", { src: "https://x.test/i.png", alt: "pic" }),
        el("img", { src: "data:image/png;base64,AA", alt: "inline" }),
        el("a", { href: "javascript:alert(1)" }, "bad"),
      ),
      el("script", {}, "ignored()"),
    )),
    "```ts\nconst a = 1;\n```\n\n| A | B\\|C |\n| --- | --- |\n| 1 | 2 |\n\n![pic](https://x.test/i.png)inlinebad",
  );
});

test("only semantic HTML is converted", () => {
  assertEquals(
    htmlHasMarkdownStructure('<div><span style="color:red">x</span></div>'),
    false,
  );
  assertEquals(htmlHasMarkdownStructure("<p>a <b>b</b></p>"), true);
  assertEquals(htmlHasMarkdownStructure('<a href="https://x">x</a>'), true);
});
