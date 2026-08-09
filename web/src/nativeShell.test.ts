import { assertEquals } from "jsr:@std/assert";
import {
  nativeClipboardImageStatus,
  nativeClipboardImageFiles,
  readNativeClipboardImages,
  supportsNativeClipboardImages,
} from "./nativeShell.ts";

Deno.test("native clipboard image payloads become ordinary image Files", async () => {
  const files = nativeClipboardImageFiles({
    changeCount: 7,
    images: [
      {
        name: "screenshot.png",
        mimeType: "image/png",
        data: btoa("png-bytes"),
      },
      { name: "not-an-image.txt", mimeType: "text/plain", data: btoa("no") },
      { name: "broken.png", mimeType: "image/png", data: "%%%" },
    ],
  });

  assertEquals(files.length, 1);
  assertEquals(files[0]?.name, "screenshot.png");
  assertEquals(files[0]?.type, "image/png");
  assertEquals(await files[0]?.text(), "png-bytes");
});

Deno.test("image paste stays disabled without both native clipboard bridges", () => {
  assertEquals(supportsNativeClipboardImages(), false);
});

Deno.test("image status probes metadata without reading clipboard payloads", async () => {
  const root = globalThis as typeof globalThis & {
    __cowboyClipboardImageStatus?: () => Promise<unknown>;
    __cowboyReadClipboardImages?: () => Promise<unknown>;
  };
  const previousStatus = root.__cowboyClipboardImageStatus;
  const previousRead = root.__cowboyReadClipboardImages;
  let payloadReads = 0;
  root.__cowboyClipboardImageStatus = () =>
    Promise.resolve({ hasImages: true, changeCount: 9 });
  root.__cowboyReadClipboardImages = () => {
    payloadReads += 1;
    return Promise.resolve({
      images: [{
        name: "clipboard.png",
        mimeType: "image/png",
        data: btoa("image"),
      }],
    });
  };

  try {
    assertEquals(supportsNativeClipboardImages(), true);
    assertEquals(await nativeClipboardImageStatus(), {
      supported: true,
      hasImages: true,
      changeCount: 9,
    });
    assertEquals(payloadReads, 0);
    assertEquals((await readNativeClipboardImages()).length, 1);
    assertEquals(payloadReads, 1);
  } finally {
    if (previousStatus === undefined) {
      delete root.__cowboyClipboardImageStatus;
    } else {
      root.__cowboyClipboardImageStatus = previousStatus;
    }
    if (previousRead === undefined) {
      delete root.__cowboyReadClipboardImages;
    } else {
      root.__cowboyReadClipboardImages = previousRead;
    }
  }
});
