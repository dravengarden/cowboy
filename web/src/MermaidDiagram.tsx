import {
  alpha,
  Box,
  CircularProgress,
  Typography,
  useTheme,
} from "@mui/material";
import OpenInFullRounded from "@mui/icons-material/OpenInFullRounded";
import type { MermaidConfig } from "mermaid";
import { type ReactNode, useEffect, useId, useMemo, useState } from "react";
import { type GalleryMedia, ImageLightbox } from "@cowboy/app-shell";
import { useReliableTouchTap } from "./useReliableTouchTap";

// The surface a dark diagram is read against: the in-page plate below and the
// lightbox's plate for a self-themed figure resolve to the same near-black
// card, so Mermaid's own opaque boxes (edge labels, subgraphs) are tuned to sit
// on it instead of cutting holes in it.
const DARK_SURFACE = "#14161c";

// Mermaid's stock dark palette paints near-black nodes, grey borders, and dim
// labels — on Cowboy's dark page (and on the lightbox's near-black backdrop)
// the whole diagram collapses into one flat smudge, which is exactly what it
// looked like on device. Lift the node fill clearly above the surface, brighten
// borders, edges, and text, and keep subgraphs a distinct recessed plane.
const DARK_THEME_VARIABLES: MermaidConfig["themeVariables"] = {
  darkMode: true,
  background: "transparent",
  mainBkg: "#2b3242",
  primaryColor: "#2b3242",
  primaryBorderColor: "#8fa3c8",
  primaryTextColor: "#f2f5fa",
  secondaryColor: "#353d51",
  tertiaryColor: "#1e2330",
  nodeBorder: "#8fa3c8",
  nodeTextColor: "#f2f5fa",
  textColor: "#e8edf7",
  titleColor: "#f2f5fa",
  lineColor: "#a9b8d6",
  clusterBkg: "#1a1e29",
  clusterBorder: "#5a6780",
  edgeLabelBackground: DARK_SURFACE,
};

type DiagramMode = "dark" | "light";

let configuredMode: DiagramMode | undefined;

async function renderMermaid(
  id: string,
  source: string,
  mode: DiagramMode,
): Promise<string> {
  const mermaid = (await import("mermaid")).default;
  if (configuredMode !== mode) {
    // initialize() rebuilds the site config from Mermaid's defaults, so the
    // dark overrides do not leak into a later light render.
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: "strict",
      theme: mode === "dark" ? "dark" : "neutral",
      ...(mode === "dark" ? { themeVariables: DARK_THEME_VARIABLES } : {}),
    });
    configuredMode = mode;
  }
  const { svg } = await mermaid.render(id, source);
  return svg;
}

function prepareInlineSvg(markup: string): string {
  // Mermaid's HTML labels live in foreignObject nodes and follow HTML parsing
  // rules (for example, <br> is not XML-self-closing). Parsing the result as
  // image/svg+xml turns an otherwise valid diagram into a parsererror document.
  // An inert template preserves those labels exactly as the successful in-page
  // render does, while still letting us validate and size the SVG root.
  const template = document.createElement("template");
  template.innerHTML = markup;
  const root = template.content.firstElementChild;
  if (!(root instanceof SVGSVGElement)) {
    throw new Error("Mermaid returned invalid SVG markup");
  }
  const viewBox = root.getAttribute("viewBox")?.trim().split(/[ ,]+/)
    .map(Number);
  if (
    viewBox?.length !== 4 ||
    !Number.isFinite(viewBox[2]) || !Number.isFinite(viewBox[3]) ||
    viewBox[2]! <= 0 || viewBox[3]! <= 0
  ) {
    throw new Error("Mermaid returned an invalid SVG viewport");
  }
  if (!root.getAttribute("xmlns")) {
    root.setAttribute("xmlns", "http://www.w3.org/2000/svg");
  }
  // Mermaid's responsive page SVG uses width="100%". Give the inline
  // lightbox copy an intrinsic size from its viewBox so normal max-width /
  // max-height containment can fit it without loading it as another image.
  root.setAttribute("width", String(Math.ceil(viewBox[2]!)));
  root.setAttribute("height", String(Math.ceil(viewBox[3]!)));
  root.style.removeProperty("max-width");
  root.style.removeProperty("width");
  root.style.removeProperty("height");
  root.style.backgroundColor = "transparent";
  return root.outerHTML;
}

export function MermaidDiagram({
  source,
  fallback,
}: {
  source: string;
  fallback?: ReactNode;
}): React.JSX.Element {
  const reactId = useId().replaceAll(":", "") || "diagram";
  const isDark = useTheme().palette.mode === "dark";
  const mode: DiagramMode = isDark ? "dark" : "light";
  const [svg, setSvg] = useState<string>();
  const [failed, setFailed] = useState(false);
  const [previewOpen, setPreviewOpen] = useState(false);
  useEffect(() => {
    let cancelled = false;
    setSvg(undefined);
    setFailed(false);
    setPreviewOpen(false);
    void renderMermaid(`cowboy-mermaid-${reactId}`, source, mode)
      .then((next) => {
        const prepared = prepareInlineSvg(next);
        if (!cancelled) setSvg(prepared);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [reactId, source, mode]);
  const previewImages = useMemo<GalleryMedia[]>(() =>
    svg
      ? [{
        kind: "inline-svg",
        markup: svg,
        alt: "Mermaid diagram",
        themed: true,
      }]
      : [], [svg]);
  const openTap = useReliableTouchTap<HTMLDivElement>(() => {
    if (previewImages.length > 0) setPreviewOpen(true);
  });
  if (failed) {
    return (
      <Box data-mermaid-source-fallback sx={{ px: 2, py: 2 }}>
        <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
          Mermaid preview unavailable. Showing the Markdown source.
        </Typography>
        {fallback ?? (
          <Box
            component="pre"
            sx={{
              m: 0,
              fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
              fontSize: "0.875rem",
              whiteSpace: "pre-wrap",
              overflowWrap: "anywhere",
            }}
          >
            {source}
          </Box>
        )}
      </Box>
    );
  }
  if (!svg) {
    return (
      <Box sx={{ display: "grid", placeItems: "center", flex: 1, py: 6 }}>
        <CircularProgress size={24} />
      </Box>
    );
  }
  return (
    <>
      <Box
        data-review-mermaid-preview
        role="button"
        tabIndex={0}
        aria-label="Open Mermaid diagram preview"
        title="Open diagram preview"
        {...openTap}
        onKeyDown={(event): void => {
          if (event.key !== "Enter" && event.key !== " ") return;
          event.preventDefault();
          setPreviewOpen(true);
        }}
        sx={{
          position: "relative",
          width: "100%",
          maxWidth: 880,
          mx: "auto",
          cursor: "zoom-in",
          borderRadius: 1,
          outline: 0,
          "&:focus-visible": {
            outline: 2,
            outlineColor: "primary.main",
            outlineOffset: 2,
          },
        }}
      >
        <Box
          sx={{
            px: 2,
            py: 2,
            overflow: "auto",
            // In dark mode the diagram sits on its own recessed card. Without
            // it the tuned node fills still read against whatever surface the
            // transcript happens to use, and a wide diagram's edges trail off
            // into the page with no figure boundary.
            ...(isDark
              ? {
                borderRadius: 1,
                bgcolor: DARK_SURFACE,
                border: 1,
                borderColor: "rgba(255, 255, 255, 0.09)",
              }
              : {}),
            "& svg": { maxWidth: "100%", height: "auto" },
          }}
          // mermaid.render() returns sanitized SVG when securityLevel is strict.
          dangerouslySetInnerHTML={{ __html: svg }}
        />
        <Box
          aria-hidden
          sx={(muiTheme) => ({
            position: "absolute",
            top: 8,
            right: 8,
            display: "grid",
            placeItems: "center",
            width: 32,
            height: 32,
            border: 1,
            borderColor: "divider",
            borderRadius: 1.5,
            color: "text.secondary",
            bgcolor: alpha(muiTheme.palette.background.paper, 0.88),
            pointerEvents: "none",
          })}
        >
          <OpenInFullRounded sx={{ fontSize: 18 }} />
        </Box>
      </Box>
      <ImageLightbox
        images={previewImages}
        index={previewOpen ? 0 : null}
        onIndex={(): void => undefined}
        onClose={(): void => setPreviewOpen(false)}
      />
    </>
  );
}
