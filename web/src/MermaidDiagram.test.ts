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

Deno.test("a zoomed diagram survives the lightbox's centring flex row", () => {
  // Zooming bakes the settled scale into the element's own width/height. An
  // inline SVG is not replaced content, so its automatic flex minimum is zero
  // and the centred item shrinks straight back to the viewport — the zoom
  // visibly springing back the moment it settled. Both media keep flexShrink
  // off; fit sizing still comes from max-width / max-height.
  assert(
    gestureSource.includes(
      "img.style.width = `${box.imageWidth * tf.current.scale}px`",
    ),
  );
  const svgBlock = lightboxSource.slice(
    lightboxSource.indexOf('"& > svg": {'),
    lightboxSource.indexOf("dangerouslySetInnerHTML={{ __html: current.markup }}"),
  );
  assert(svgBlock.includes("flexShrink: 0"));
  assert(svgBlock.includes('maxWidth: "100%"'));
  const imgBlock = lightboxSource.slice(
    lightboxSource.indexOf("<img"),
    lightboxSource.indexOf("One bottom dock holds every control"),
  );
  assert(imgBlock.includes("flexShrink: 0"));
});

Deno.test("dark diagrams are legible inline and enlarged", () => {
  // Mermaid's stock dark palette is near-black on near-black. The overrides
  // lift node fills, borders, edges, and label text off the surface, and the
  // in-page figure gets the same plate the lightbox gives a self-themed image.
  assert(mermaidSource.includes('theme: mode === "dark" ? "dark" : "neutral"'));
  assert(
    mermaidSource.includes(
      '...(mode === "dark" ? { themeVariables: DARK_THEME_VARIABLES } : {})',
    ),
  );
  for (
    const key of [
      "mainBkg",
      "nodeBorder",
      "nodeTextColor",
      "lineColor",
      "clusterBkg",
      "clusterBorder",
      "edgeLabelBackground",
    ]
  ) {
    assert(
      mermaidSource.includes(`${key}:`),
      `dark theme variable ${key} is missing`,
    );
  }
  assert(mermaidSource.includes("bgcolor: DARK_SURFACE"));
  // A self-themed figure is never inverted — the palette above is already
  // mode-correct, so a second correction would flip it back to light.
  assert(lightboxSource.includes("const invertPlate = plate && !selfThemed && isDarkMode"));
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
