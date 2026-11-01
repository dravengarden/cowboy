import { assert, assertEquals } from "@std/assert";
import { afterAll, beforeAll, test } from "bun:test";
import {
  imageDisplayUrl,
  isProtectedImageSource,
  remoteImageHost,
  setProtectedImageSource,
} from "./protectedImage.ts";

const previousLocation = globalThis.location;
beforeAll(() => {
  Object.defineProperty(globalThis, "location", {
    configurable: true,
    value: new URL("https://cowboy.example/"),
  });
});
afterAll(() => {
  Object.defineProperty(globalThis, "location", { configurable: true, value: previousLocation });
});

test("sources that stay on this origin or device load in place", () => {
  for (
    const source of [
      "/api/artifacts/a.png",
      "/logo.png",
      `${globalThis.location.origin}/x.png`,
      "data:image/png;base64,AAAA",
      "blob:https://cowboy.example/1",
    ]
  ) {
    assertEquals(remoteImageHost(source), null, source);
  }
});

test("a source that contacts another host is named, and an invalid one is refused", () => {
  assertEquals(remoteImageHost("https://evil.example:8443/x?secret=1"), "evil.example:8443");
  assertEquals(remoteImageHost("//evil.example/x"), "evil.example");
  // A percent-encoded host survives Markdown and throws in `new URL`. It must
  // not take the renderer down with it.
  assertEquals(remoteImageHost("http://%5B"), "");
  assertEquals(isProtectedImageSource("http://%5B"), false);
});

test("an image element is never pointed at a remote source", () => {
  const image = { src: "", removeAttribute() {} } as unknown as HTMLImageElement;
  setProtectedImageSource(image, "https://evil.example/x.png");
  assertEquals(image.src, "");
  setProtectedImageSource(image, "data:image/gif;base64,AAAA");
  assertEquals(image.src, "data:image/gif;base64,AAAA");
});

test("bytes that can carry script never become a same-origin blob URL", async () => {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>`;
  for (const type of ["image/svg+xml", "IMAGE/SVG+XML; charset=utf-8", "text/html", ""]) {
    const { url } = await imageDisplayUrl(new Blob([svg], { type }));
    assert(url.startsWith("data:"), `${type}: ${url}`);
  }
  const { url } = await imageDisplayUrl(new Blob([svg], { type: "image/svg+xml" }));
  assertEquals(atob(url.split(",")[1]!), svg);
});

test("raster bytes keep the cheap revocable blob URL", async () => {
  const { url, release } = await imageDisplayUrl(new Blob([new Uint8Array(4)], { type: "image/png" }));
  assert(url.startsWith("blob:"));
  release();
});
