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
