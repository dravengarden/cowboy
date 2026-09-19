import { assert, assertEquals } from "jsr:@std/assert";

const mermaidSource = await Deno.readTextFile(
  new URL("./MermaidDiagram.tsx", import.meta.url),
);
const markdownSource = await Deno.readTextFile(
  new URL("./MarkdownImpl.tsx", import.meta.url),
);
const lightboxSource = await Deno.readTextFile(
  new URL("../../components/app-shell/image-lightbox.tsx", import.meta.url),
);
const gestureSource = await Deno.readTextFile(
  new URL(
    "../../components/app-shell/image-lightbox-gestures.ts",
    import.meta.url,
  ),
);

Deno.test("Mermaid lightbox preserves HTML labels as inline SVG", () => {
  assert(mermaidSource.includes('document.createElement("template")'));
  assertEquals(mermaidSource.includes("new DOMParser"), false);
  assertEquals(mermaidSource.includes("data:image/svg+xml"), false);
  assert(mermaidSource.includes('kind: "inline-svg"'));
  assert(lightboxSource.includes('current.kind === "inline-svg"'));
  assert(
    lightboxSource.includes(
      "dangerouslySetInnerHTML={{ __html: current.markup }}",
    ),
  );
  assert(gestureSource.includes("HTMLImageElement | SVGSVGElement"));
});

Deno.test("Mermaid failures return to the ordinary Markdown code renderer", () => {
  assert(mermaidSource.includes("data-mermaid-source-fallback"));
  const branchStart = markdownSource.indexOf(
    'if (lang.toLowerCase() === "mermaid")',
  );
  assert(branchStart >= 0);
  const mermaidBranch = markdownSource.slice(branchStart, branchStart + 1_200);
  assert(mermaidBranch.includes("fallback={"));
  assert(mermaidBranch.includes("<MarkdownCodeBoundary"));
  assert(mermaidBranch.includes("<CodeBlock"));
});
