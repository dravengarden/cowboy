import { assertEquals } from "jsr:@std/assert";
import { normalizeMarkdownMath } from "./markdownMath.ts";

Deno.test("LaTeX math delimiters normalize to GitHub math syntax", () => {
  assertEquals(
    normalizeMarkdownMath(
      String.raw`Inline \(a_t+b_t\), then:
\[
m_t = \frac{a_t+b_t}{2}
\]`,
    ),
    `Inline $a_t+b_t$, then:
$$
m_t = \\frac{a_t+b_t}{2}
$$`,
  );
});

Deno.test("math-looking code and unmatched delimiters stay exact", () => {
  const source = String.raw`Use \[ as text.

~~~tex
\[
x = \frac{1}{2}
\]
~~~

Inline ` + "`\\(not_math\\)`" + String.raw` and escaped \\\[literal\\\].`;
  assertEquals(normalizeMarkdownMath(source), source);
});

Deno.test("plain-language boxed display math becomes a readable callout", () => {
  assertEquals(
    normalizeMarkdownMath(String.raw`\[
\boxed{
Hyperliquid公共L2 WS不是10ms级逐订单流；
其快照推送节奏约500ms，加上网络与处理后通常应按数百毫秒级建模。
}
\]`),
    `> **Hyperliquid公共L2 WS不是10ms级逐订单流；**
> **其快照推送节奏约500ms，加上网络与处理后通常应按数百毫秒级建模。**`,
  );
});

Deno.test("real TeX boxed expressions remain math", () => {
  const source = String.raw`\[
\boxed{\text{Order-Book Confirmed Trend Following}}
\]`;
  assertEquals(
    normalizeMarkdownMath(source),
    `$$
\\boxed{\\text{Order-Book Confirmed Trend Following}}
$$`,
  );
});
