import {
  alpha,
  Box,
  CircularProgress,
  Typography,
  useTheme,
} from "@mui/material";
import OpenInFullRounded from "@mui/icons-material/OpenInFullRounded";
import type { MermaidConfig } from "mermaid";
import { useEffect, useId, useMemo, useState } from "react";
import { type GalleryImage, ImageLightbox } from "@cowboy/app-shell";
import { useReliableTouchTap } from "./useReliableTouchTap";

let configuredTheme: MermaidConfig["theme"];

async function renderMermaid(
  id: string,
  source: string,
  theme: NonNullable<MermaidConfig["theme"]>,
): Promise<string> {
  const mermaid = (await import("mermaid")).default;
  if (configuredTheme !== theme) {
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: "strict",
      theme,
    });
    configuredTheme = theme;
  }
  const { svg } = await mermaid.render(id, source);
  return svg;
}

function svgDataUrl(markup: string): string {
  const document = new DOMParser().parseFromString(markup, "image/svg+xml");
  const root = document.documentElement;
  if (root.localName === "svg") {
    if (!root.getAttribute("xmlns")) {
      root.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    }
    const viewBox = root.getAttribute("viewBox")?.trim().split(/[ ,]+/)
      .map(Number);
    if (
      viewBox?.length === 4 && Number.isFinite(viewBox[2]) &&
      Number.isFinite(viewBox[3]) && viewBox[2]! > 0 && viewBox[3]! > 0
    ) {
      // Mermaid emits width="100%" for the in-page responsive figure. An SVG
      // loaded through <img> has no containing block for that percentage, so
      // pin its intrinsic size to the diagram's own viewBox before the shared
      // lightbox fits and zooms it.
      root.setAttribute("width", String(Math.ceil(viewBox[2]!)));
      root.setAttribute("height", String(Math.ceil(viewBox[3]!)));
      root.style.removeProperty("max-width");
      root.style.removeProperty("width");
      root.style.removeProperty("height");
    }
    root.style.backgroundColor = "transparent";
    markup = new XMLSerializer().serializeToString(root);
  }
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(markup)}`;
}

export function MermaidDiagram({
  source,
}: {
  source: string;
}): React.JSX.Element {
  const reactId = useId().replaceAll(":", "") || "diagram";
  const theme = useTheme().palette.mode === "dark" ? "dark" : "neutral";
  const [svg, setSvg] = useState<string>();
  const [failed, setFailed] = useState(false);
  const [previewOpen, setPreviewOpen] = useState(false);
  useEffect(() => {
    let cancelled = false;
    setSvg(undefined);
    setFailed(false);
    setPreviewOpen(false);
    void renderMermaid(`cowboy-mermaid-${reactId}`, source, theme)
      .then((next) => {
        if (!cancelled) setSvg(next);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [reactId, source, theme]);
  const previewImages = useMemo<GalleryImage[]>(() =>
    svg
      ? [{
        src: svgDataUrl(svg),
        alt: "Mermaid diagram",
        themed: true,
      }]
      : [], [svg]);
  const openTap = useReliableTouchTap<HTMLDivElement>(() => {
    if (previewImages.length > 0) setPreviewOpen(true);
  });
  if (failed) {
    return (
      <Box sx={{ px: 2, py: 2 }}>
        <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
          Couldn’t render this Mermaid diagram. Showing the source.
        </Typography>
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
