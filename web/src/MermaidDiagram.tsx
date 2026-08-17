import { Box, CircularProgress, Typography, useTheme } from "@mui/material";
import type { MermaidConfig } from "mermaid";
import { useEffect, useId, useState } from "react";

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

export function MermaidDiagram({
  source,
}: {
  source: string;
}): React.JSX.Element {
  const reactId = useId().replaceAll(":", "") || "diagram";
  const theme = useTheme().palette.mode === "dark" ? "dark" : "neutral";
  const [svg, setSvg] = useState<string>();
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let cancelled = false;
    setSvg(undefined);
    setFailed(false);
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
    <Box
      data-review-mermaid-preview
      sx={{
        width: "100%",
        maxWidth: 880,
        mx: "auto",
        px: 2,
        py: 2,
        overflow: "auto",
        "& svg": { maxWidth: "100%", height: "auto" },
      }}
      // mermaid.render() returns sanitized SVG when securityLevel is strict.
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}
