import { assertEquals } from "jsr:@std/assert";
import {
  htmlIsOnlyImage,
  markdownLinkForPastedUrl,
  normalizeClipboardText,
  pastedTextBeatsFiles,
} from "./clipboardPastePolicy";

function clipboard(data: Record<string, string>): Pick<DataTransfer, "getData"> {
  return { getData: (type: string): string => data[type] ?? "" };
}

Deno.test("clipboard text is normalized to the document line separator", () => {
  assertEquals(normalizeClipboardText("a\r\nb\rc\n"), "a\nb\nc\n");
});

Deno.test("a copied picture's HTML does not beat its file", () => {
  assertEquals(
    htmlIsOnlyImage(`<meta charset="utf-8"><img src="https://x/y.png" alt="y">`),
    true,
  );
  assertEquals(
    htmlIsOnlyImage("<html><body><!--StartFragment--><img src=a><!--EndFragment--></body></html>"),
    true,
  );
  assertEquals(htmlIsOnlyImage("<table><tr><td>1</td></tr></table>"), false);
  assertEquals(
    pastedTextBeatsFiles(clipboard({ "text/html": "<img src=a>", "text/plain": "" })),
    false,
  );
});

Deno.test("office rich text beats the rendered table picture", () => {
  assertEquals(
    pastedTextBeatsFiles(clipboard({
      "text/html": "<table><tr><td>1</td></tr></table>",
      "text/plain": "1",
    })),
    true,
  );
  // A Finder file copy has a file name as plain text but no HTML.
  assertEquals(
    pastedTextBeatsFiles(clipboard({ "text/plain": "report.pdf" })),
    false,
  );
});

Deno.test("a URL pasted over a single-line selection becomes a link", () => {
  assertEquals(
    markdownLinkForPastedUrl("docs", "https://example.com/a"),
    "[docs](https://example.com/a)",
  );
  assertEquals(markdownLinkForPastedUrl("", "https://example.com"), null);
  assertEquals(markdownLinkForPastedUrl("a\nb", "https://example.com"), null);
  assertEquals(markdownLinkForPastedUrl("docs", "not a url"), null);
  assertEquals(markdownLinkForPastedUrl("docs", "https://a\nhttps://b"), null);
});
