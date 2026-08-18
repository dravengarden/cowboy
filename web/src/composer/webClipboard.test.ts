import { assertEquals } from "jsr:@std/assert";
import { readWebClipboard, supportsWebClipboardRead } from "./webClipboard.ts";

Deno.test("web clipboard read is offered when the browser exposes the API", () => {
  assertEquals(
    supportsWebClipboardRead({
      read: () => Promise.resolve([]),
      readText: () => Promise.resolve(""),
    }),
    true,
  );
  assertEquals(
    supportsWebClipboardRead({
      readText: () => Promise.resolve("hi"),
    } as Pick<Clipboard, "read" | "readText">),
    true,
  );
  assertEquals(supportsWebClipboardRead(undefined), false);
});

Deno.test("web clipboard prefers image items and keeps accompanying text", async () => {
  const png = new Blob(["png"], { type: "image/png" });
  const text = new Blob(["hello"], { type: "text/plain" });
  const contents = await readWebClipboard({
    read: () =>
      Promise.resolve([
        {
          types: ["image/png", "text/plain"],
          getType: (type: string) =>
            Promise.resolve(type === "image/png" ? png : text),
        } as ClipboardItem,
      ]),
    readText: () => Promise.resolve("ignored"),
  });
  assertEquals(contents.files.length, 1);
  assertEquals(contents.files[0]?.type, "image/png");
  assertEquals(contents.text, "hello");
});

Deno.test("web clipboard falls back to readText when read() is rejected", async () => {
  const contents = await readWebClipboard({
    read: () => Promise.reject(new Error("NotAllowedError")),
    readText: () => Promise.resolve("plain"),
  });
  assertEquals(contents.files, []);
  assertEquals(contents.text, "plain");
});
