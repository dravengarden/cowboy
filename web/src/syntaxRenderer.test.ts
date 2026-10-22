import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { PrismLight } from "react-syntax-highlighter";
import markup from "react-syntax-highlighter/dist/esm/languages/prism/markup";
import tsx from "react-syntax-highlighter/dist/esm/languages/prism/tsx";
import diff from "react-syntax-highlighter/dist/esm/languages/prism/diff";
import {
  oneDark,
  oneLight,
} from "react-syntax-highlighter/dist/esm/styles/prism";
import { cachedInlineRenderer, inlineTokenProps } from "./syntaxRenderer.ts";

PrismLight.registerLanguage("markup", markup);
PrismLight.registerLanguage("tsx", tsx);
PrismLight.registerLanguage("diff", diff);

const samples: [string, string][] = [
  [
    "markup",
    "<div class=\"x\" data-id='1'>\n  <!-- note --><span>text &amp; more</span>\n</div>",
  ],
  [
    "tsx",
    "export const App = ({ n }: { n: number }) => <b className={`v${n}`}>{n * 2}</b>; // done",
  ],
  ["diff", "@@ -1,2 +1,2 @@\n-old line\n+new line\n context"],
];

test("cached rendering matches the library renderer byte for byte", () => {
  for (const theme of [oneLight, oneDark]) {
    for (const [language, code] of samples) {
      for (const wrapLines of [false, true]) {
        const props = {
          language,
          style: theme,
          wrapLines,
          showLineNumbers: wrapLines,
          lineNumberStyle: wrapLines ? { display: "none" } : undefined,
          lineProps: wrapLines
            ? (line: number) => ({
              "data-diff-sign": String(line % 2),
              style: { display: "block", background: "red" },
            })
            : undefined,
        };
        const expected = renderToStaticMarkup(
          createElement(PrismLight, props, code),
        );
        const actual = renderToStaticMarkup(
          createElement(PrismLight, {
            ...props,
            renderer: cachedInlineRenderer,
          }, code),
        );
        assertEquals(
          actual,
          expected,
          `${language} wrapLines=${String(wrapLines)}`,
        );
      }
    }
  }
});

test("each class list is computed once per theme", () => {
  const first = inlineTokenProps(["token", "tag", "attr-name"], oneLight);
  assert(first === inlineTokenProps(["token", "tag", "attr-name"], oneLight));
  assert(first !== inlineTokenProps(["token", "tag", "attr-name"], oneDark));
});
