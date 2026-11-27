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

  // Semantic HTML without dark-mode styles or broken file:// hrefs
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

  // Clean readable plain text without Markdown punctuation clutter
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
