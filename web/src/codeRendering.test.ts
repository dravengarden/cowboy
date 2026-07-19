import {
  HIGHLIGHT_CHAR_LIMIT,
  HIGHLIGHT_LINE_LIMIT,
  chunkCodeForRendering,
  previewCodeForRendering,
  shouldUseLightweightCode,
} from "./codeRendering";

Deno.test("large code falls back before syntax highlighting explodes the DOM", () => {
  if (shouldUseLightweightCode("const compact = true;\n")) {
    throw new Error("ordinary code should retain syntax highlighting");
  }
  if (!shouldUseLightweightCode("x".repeat(HIGHLIGHT_CHAR_LIMIT + 1))) {
    throw new Error("large single-line output should use the lightweight renderer");
  }
  if (!shouldUseLightweightCode("x\n".repeat(HIGHLIGHT_LINE_LIMIT + 1))) {
    throw new Error("large multi-line output should use the lightweight renderer");
  }
});

Deno.test("large code preview stops at a complete line", () => {
  const source = "one\ntwo\nthree\nfour";
  if (previewCodeForRendering(source, 2) !== "one\ntwo") {
    throw new Error("preview should contain exactly the requested complete lines");
  }
  if (previewCodeForRendering(source, 20) !== source) {
    throw new Error("a short source should remain untouched");
  }
});

Deno.test("lightweight code chunks preserve every line in order", () => {
  const source = Array.from({ length: 11 }, (_, index) => `line ${index}`).join("\n");
  const chunks = chunkCodeForRendering(source, 3);
  if (chunks.join("\n") !== source) {
    throw new Error("chunking changed the displayed source");
  }
});
