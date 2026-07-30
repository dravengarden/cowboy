import type { CodeHoverBlock } from "./codeApi";

function cleanRustdocFence(markdown: string): string {
  const lines = markdown.replaceAll(/<br\s*\/?>/gi, "\n").split("\n");
  let inFence = false;
  let rustFence = false;
  return lines.flatMap((line): string[] => {
    const fence = line.match(/^\s*```+\s*([\w+-]*)/);
    if (fence) {
      if (inFence) {
        inFence = false;
        rustFence = false;
      } else {
        inFence = true;
        rustFence = ["", "rs", "rust"].includes(
          fence[1]?.toLowerCase() ?? "",
        );
      }
      return [line];
    }
    // rustdoc hides setup lines prefixed with `#` inside Rust examples. The
    // language server returns the source markdown, so apply rustdoc's display
    // semantics before handing it to Cowboy's general Markdown renderer.
    if (rustFence && /^\s*#(?:\s.*)?$/.test(line)) return [];
    return [line];
  }).join("\n").replaceAll(/\n{3,}/g, "\n\n").trim();
}

export function presentHoverBlock(block: CodeHoverBlock): CodeHoverBlock {
  if (!block.markdown) return block;
  return { ...block, text: cleanRustdocFence(block.text) };
}
