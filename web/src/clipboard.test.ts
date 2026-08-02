import { assertEquals } from "jsr:@std/assert";
import { readComposerClipboard } from "./clipboard.ts";

Deno.test("composer clipboard prefers one image representation per item", async () => {
  const content = await readComposerClipboard({
    read: () =>
      Promise.resolve([
        {
          types: ["image/png", "text/html"],
          getType: (type: string) =>
            Promise.resolve(
              new Blob([type === "image/png" ? "png" : "html"], { type }),
            ),
        },
      ]),
  });

  assertEquals(content.text, "");
  assertEquals(content.files.length, 1);
  assertEquals(content.files[0]?.name, "pasted-file.png");
  assertEquals(content.files[0]?.type, "image/png");
});

Deno.test("composer clipboard reads plain text and supports readText fallback", async () => {
  const rich = await readComposerClipboard({
    read: () =>
      Promise.resolve([
        {
          types: ["text/plain", "text/html"],
          getType: (type: string) =>
            Promise.resolve(
              new Blob([type === "text/plain" ? "hello" : "<b>hello</b>"], {
                type,
              }),
            ),
        },
      ]),
  });
  assertEquals(rich, { text: "hello", files: [] });

  const fallback = await readComposerClipboard({
    readText: () => Promise.resolve("fallback"),
  });
  assertEquals(fallback, { text: "fallback", files: [] });
});
