import { test } from "bun:test";
import { assertEquals, assertStringIncludes } from "@std/assert";
import { prismThemeCss } from "./prismThemeCss.ts";

test("token keys become scoped token class selectors with CSS declarations", () => {
  const css = prismThemeCss({
    keyword: { color: "#a626a4", fontWeight: 700 },
    "keyword.module": { fontStyle: "italic" },
    'pre[class*="language-"]': {
      background: "#fafafa",
      MozTabSize: 2,
      padding: 0,
    },
    ".language-css .token.string, .style .token.string": { color: "#50a14f" },
    empty: {},
    'pre[class*="language-"]::selection': { background: "#e5e5e6" },
  }, "scope");
  const rules = css.split("\n");
  assertEquals(rules.length, 4);
  assertEquals(
    rules[0],
    ".scope .token.keyword{color:#a626a4;font-weight:700}",
  );
  assertEquals(rules[1], ".scope .token.keyword.module{font-style:italic}");
  assertEquals(
    rules[2],
    '.scope pre[class*="language-"]{background:#fafafa;-moz-tab-size:2;padding:0}',
  );
  assertStringIncludes(
    rules[3]!,
    ".scope .language-css .token.string,.scope .style .token.string{color:#50a14f}",
  );
});
