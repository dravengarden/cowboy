import { expect, test } from "bun:test";
import { COMPACTING_NOTICE, type RenderItem } from "./derive.ts";
import {
  extractCopyableMessage,
  extractMessageMarkdown,
  formatMessageForCopy,
} from "./messageCopyFormat.ts";

test("extractMessageMarkdown keeps verbatim markdown and omits binary attachment chunks", () => {
  const md = extractMessageMarkdown([
    { type: "text", text: "## Summary\n\n- **Item 1**" },
    { type: "image", src: "/api/artifact/abc", alt: "screenshot" },
    { type: "file", name: "report.pdf", mimeType: "application/pdf" },
    { type: "text", text: "Follow-up line" },
  ]);
  expect(md).toBe("## Summary\n\n- **Item 1**\n\nFollow-up line");
});

test("extractCopyableMessage admits assistant & human user bubbles and excludes system/compacting notices", () => {
  const assistant: RenderItem = {
    key: "1",
    kind: "message",
    role: "assistant",
    chunks: [{ type: "text", text: "Hello **world**" }],
  };
  expect(extractCopyableMessage(assistant)).toEqual({
    markdown: "Hello **world**",
    role: "assistant",
  });

  const compacting: RenderItem = {
    key: "2",
    kind: "message",
    role: "assistant",
    chunks: [{ type: "text", text: COMPACTING_NOTICE }],
  };
  expect(extractCopyableMessage(compacting)).toBeNull();

  const humanUser: RenderItem = {
    key: "3",
    kind: "message",
    role: "user",
    chunks: [{ type: "text", text: "Please fix `foo_bar_baz`" }],
  };
  expect(extractCopyableMessage(humanUser)).toEqual({
    markdown: "Please fix `foo_bar_baz`",
    role: "user",
  });

  const automatedPrompt: RenderItem = {
    key: "4",
    kind: "message",
    role: "user",
    origin: { actor: "cowboy", source: "schedule" },
    chunks: [{ type: "text", text: "Scheduled check" }],
  };
  expect(extractCopyableMessage(automatedPrompt)).toBeNull();

  const imageOnly: RenderItem = {
    key: "5",
    kind: "message",
    role: "user",
    chunks: [{ type: "image", src: "blob:123" }],
  };
  expect(extractCopyableMessage(imageOnly)).toBeNull();
});

test("formatMessageForCopy renders headings, lists, task lists, nested items, blockquotes, and GFM tables cleanly", () => {
  const md = [
    "### 一、现状诊断与方案",
    "",
    "- **高亮状态**：去掉 `inset 0 2px` 紫条，保留 `snake_case_name`",
    "  - 子项 1：[App.tsx](file:///home/draven/App.tsx) 和 [Docs](https://example.com/docs)",
    "- [x] 已完成",
    "- [ ] 待确认",
    "",
    "| 对标工具 | 核心机制 |",
    "| --- | :---: |",
    "| **Zed** | `All Changes` 视图 |",
    "| Antigravity | 行内批注 \\| 批量回传 |",
    "",
    "> 引用说明：*斜体* 与 ~~删除线~~",
  ].join("\n");

  const { html, plain } = formatMessageForCopy(md);

  expect(html).toContain("<h3>一、现状诊断与方案</h3>");
  expect(html).toContain("<strong>高亮状态</strong>");
  expect(html).toContain("<code>snake_case_name</code>");
  expect(html).toContain("<code>App.tsx</code>");
  expect(html).not.toContain("file:///");
  expect(html).toContain('<a href="https://example.com/docs">Docs</a>');
  expect(html).toContain("<li>☑ 已完成</li>");
  expect(html).toContain("<li>☐ 待确认</li>");
  expect(html).toContain("<table");
  expect(html).toContain("<strong>Zed</strong>");
  expect(html).toContain("<code>All Changes</code>");
  expect(html).toContain("行内批注 | 批量回传");
  expect(html).toContain("<blockquote><p>引用说明：<em>斜体</em> 与 <del>删除线</del></p></blockquote>");

  expect(plain).toContain("一、现状诊断与方案");
  expect(plain).not.toContain("###");
  expect(plain).not.toContain("**");
  expect(plain).toContain("snake_case_name");
  expect(plain).toContain("Docs (https://example.com/docs)");
  expect(plain).toContain("☑ 已完成");
  expect(plain).toContain("☐ 待确认");
  expect(plain).toContain("对标工具\t核心机制");
  expect(plain).toContain("Zed\tAll Changes 视图");
  expect(plain).toContain("> 引用说明：斜体 与 删除线");
});

test("formatMessageForCopy handles R1 edge cases: mismatched table columns, bounded parsing, table terminators, escaped pipes in code, inline triple backticks", () => {
  // Finding 1: mismatched header/delimiter column count must NEVER hang
  const mismatchedTable = formatMessageForCopy("| A | B |\n| --- | --- | --- |\n| 1 | 2 |");
  expect(mismatchedTable.plain).toContain("| A | B |");
  expect(mismatchedTable.html).not.toContain("<table");

  // Finding 2: pathological unmatched brackets, asterisks, and deep blockquotes stay fast & stack-safe
  const start = Date.now();
  const pathological = `${"[".repeat(8000)} ${"*".repeat(8000)}\n\n${"> ".repeat(200)}deep quote`;
  const bounded = formatMessageForCopy(pathological);
  expect(Date.now() - start).toBeLessThan(500);
  expect(bounded.plain).toContain("deep quote");

  // Finding 4: table terminates on list item containing |, and \| inside inline code unescapes per GFM
  const tableThenList = formatMessageForCopy([
    "| Expr | Meaning |",
    "| --- | --- |",
    "| `a \\| b` | bitwise OR |",
    "- follow-up item with | pipe",
  ].join("\n"));
  expect(tableThenList.html).toContain("<code>a | b</code>");
  expect(tableThenList.html).toContain("<ul><li>follow-up item with | pipe</li></ul>");
  expect(tableThenList.plain).toContain("a | b\tbitwise OR");
  expect(tableThenList.plain).toContain("- follow-up item with | pipe");

  // Finding 5: line-opening ```inline``` code span does NOT swallow the rest of the reply
  const inlineTriple = formatMessageForCopy("```inline code``` and **bold**\n\nNext paragraph.");
  expect(inlineTriple.html).toContain("<code>inline code</code>");
  expect(inlineTriple.html).toContain("<strong>bold</strong>");
  expect(inlineTriple.html).toContain("<p>Next paragraph.</p>");
  expect(inlineTriple.plain).toContain("inline code and bold");
  expect(inlineTriple.plain).toContain("Next paragraph.");
});

test("formatMessageForCopy preserves list code indentation, nested children across blank lines, links, LaTeX, and empty-alt images", () => {
  // Finding 6: Python indentation inside fenced code in list + nested list across blank lines
  const listWithCode = formatMessageForCopy([
    "- Python example:",
    "",
    "  ```python",
    "  def greet(name):",
    "      return f'hi {name}'",
    "  ```",
    "",
    "  - Nested child after blank line",
  ].join("\n"));
  expect(listWithCode.html).toContain("<pre><code>def greet(name):\n    return f'hi {name}'</code></pre>");
  expect(listWithCode.html).toContain("<ul><li>Nested child after blank line</li></ul>");
  expect(listWithCode.plain).toContain("      return f'hi {name}'");
  expect(listWithCode.plain).toContain("  - Nested child after blank line");

  // Finding 7: HTML entities & escaped parens in links; prose [Plan](read the attached document) stays intact
  const links = formatMessageForCopy(
    'See [Docs](https://example.com/a?x=1&amp;y=2), [Wiki](https://en.wikipedia.org/wiki/Foo_\\(bar\\)), and [Plan](read the attached document).',
  );
  expect(links.html).toContain('<a href="https://example.com/a?x=1&amp;y=2">Docs</a>');
  expect(links.html).not.toContain("&amp;amp;");
  expect(links.html).toContain('<a href="https://en.wikipedia.org/wiki/Foo_(bar)">Wiki</a>');
  expect(links.plain).toContain("[Plan](read the attached document)");

  // Finding 14: LaTeX \(...\) and \[...\] preserve backslashes and formulas
  const latex = formatMessageForCopy("Inline \\(\\frac{a}{b}\\) and spaced \\( x + y \\)\n\n\\[\\sum_{i=1}^n i\\]");
  expect(latex.plain).toContain("\\frac{a}{b}");
  expect(latex.plain).toContain("x + y");
  expect(latex.plain).toContain("\\sum_{i=1}^n i");
  expect(latex.html).toContain("\\frac{a}{b}");

  // Finding 15: empty-alt image never produces empty clipboard text
  const emptyImg = formatMessageForCopy("![](https://example.com/a.png)");
  expect(emptyImg.plain).toBe("[Image]");
  expect(emptyImg.html).toContain("[Image]");
});

test("formatMessageForCopy preserves fenced code verbatim, neutralizes XSS/unsafe links, and avoids snake_case italic corruption", () => {
  const md = [
    "Check my_var_name and [bad](javascript:alert(1)) and inline math $x^2 + y^2$.",
    "",
    "```ts",
    "const html = `<script>alert(1)</script>`;",
    "const bold = **not_bold**;",
    "```",
  ].join("\n");

  const { html, plain } = formatMessageForCopy(md);
  expect(html).toContain("my_var_name");
  expect(html).not.toContain("<em>var</em>");
  expect(html).not.toContain("javascript:");
  expect(html).toContain("bad");
  expect(html).toContain("x^2 + y^2");
  expect(html).not.toContain("<script>");
  expect(html).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
  expect(plain).toContain("const html = `<script>alert(1)</script>`;");
  expect(plain).toContain("const bold = **not_bold**;");
});

test("formatMessageForCopy handles R2 parser findings: sparse table output cap, incomplete link budget, unclosed list fence, backslashes, entities, unmatched LaTeX, and valid GFM tables", () => {
  // R2-1: sparse wide table (500 cols x 300 short rows) must cap output cleanly instead of expanding to megabytes
  const wideHeader = `| ${Array.from({ length: 300 }, (_, i) => `C${String(i)}`).join(" | ")} |`;
  const wideSep = `| ${Array.from({ length: 300 }, () => "-").join(" | ")} |`;
  const sparseRows = Array.from({ length: 200 }, () => "| x |").join("\n");
  const t0 = Date.now();
  const sparse = formatMessageForCopy(`${wideHeader}\n${wideSep}\n${sparseRows}`);
  expect(Date.now() - t0).toBeLessThan(250);
  expect(sparse.html.length).toBeLessThan(100_000);
  expect(sparse.html).not.toContain("<table");

  // R2-2: many incomplete link destinations stay bounded under parse budget
  const t1 = Date.now();
  const badLinks = "[a](http://example.com ".repeat(1500);
  const boundedLinks = formatMessageForCopy(badLinks);
  expect(Date.now() - t1).toBeLessThan(300);
  expect(boundedLinks.plain.length).toBeGreaterThan(0);

  // R2-4: unclosed fence inside a list item does NOT absorb the next unindented sibling bullet
  const unclosedListFence = formatMessageForCopy([
    "- First item:",
    "  ```ts",
    "  const a = 1;",
    "- Second sibling item",
  ].join("\n"));
  expect(unclosedListFence.html).toContain("<li>Second sibling item</li>");
  expect(unclosedListFence.html).not.toContain("Second sibling item</code>");

  // R2-5: trailing backslash in Windows path and multiline code span stay intact; prose hard breaks work
  const backslashes = formatMessageForCopy("Path is C:\\\n\nCode `foo\\\nbar` and line1\\\nline2");
  expect(backslashes.plain).toContain("Path is C:\\");
  expect(backslashes.plain).toContain("foo\\ bar");
  expect(backslashes.html).toContain("<code>foo\\ bar</code>");
  expect(backslashes.html).toContain("line1<br>line2");

  // R2-6: single-pass destination decoding resolves &copy; without double-decoding \&amp;
  const entities = formatMessageForCopy(
    "[Copy](https://example.com/?s=&copy;) and [Esc](https://example.com/?q=\\&amp;x=1)",
  );
  expect(entities.html).toContain("https://example.com/?s=%C2%A9");
  expect(entities.html).toContain("https://example.com/?q=&amp;amp;x=1");

  // R2-9: unmatched \[ does NOT swallow remaining prose into a code block
  const unmatchedLatex = formatMessageForCopy("\\[\nThis stays **normal** prose.");
  expect(unmatchedLatex.html).not.toContain("<pre><code>");
  expect(unmatchedLatex.html).toContain("<strong>normal</strong>");

  // R2-10: single-dash GFM delimiters and sparse 1-cell body rows preserve table structure
  const gfmTable = formatMessageForCopy([
    "| A | B |",
    "| - | - |",
    "only-first-col",
    "| r2a | r2b |",
  ].join("\n"));
  expect(gfmTable.html).toContain("<table");
  expect(gfmTable.html).toContain("only-first-col");
  expect(gfmTable.html).toContain("r2b");
  expect(gfmTable.plain).toContain("A\tB\nonly-first-col\t\nr2a\tr2b");
});
