import { Box, CircularProgress, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { reviewMediaUrl, type ReviewPreviewKind } from "./reviewPreview";

export function ReviewMediaPreview({
  sessionId,
  path,
  kind,
}: {
  sessionId: string;
  path: string;
  kind: Extract<ReviewPreviewKind, "image" | "svg">;
}): React.JSX.Element {
  const [failed, setFailed] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const src = reviewMediaUrl(sessionId, path);
  const name = path.split("/").at(-1) ?? path;
  if (failed) {
    return (
      <Stack
        role="alert"
        alignItems="center"
        justifyContent="center"
        spacing={1}
        sx={{ flex: 1, px: 3, textAlign: "center" }}
      >
        <Typography variant="subtitle1" fontWeight={750}>
          Couldn’t preview this {kind === "svg" ? "SVG" : "image"}
        </Typography>
        <Typography variant="body2" color="text.secondary">
          {name} could not be loaded from the current worktree.
        </Typography>
      </Stack>
    );
  }
  return (
    <Box
      data-review-media-preview={kind}
      sx={{
        position: "relative",
        flex: 1,
        minHeight: 0,
        display: "grid",
        placeItems: "center",
        px: 2,
        py: 2,
        overflow: "auto",
      }}
    >
      {!loaded && <CircularProgress size={24} sx={{ position: "absolute" }} />}
      <Box
        component="img"
        src={src}
        alt={name}
        onLoad={() => setLoaded(true)}
        onError={() => setFailed(true)}
        sx={{
          maxWidth: "100%",
          maxHeight: "100%",
          objectFit: "contain",
          visibility: loaded ? "visible" : "hidden",
          backgroundImage:
            "linear-gradient(45deg, rgba(127,127,127,0.12) 25%, transparent 25%), linear-gradient(-45deg, rgba(127,127,127,0.12) 25%, transparent 25%), linear-gradient(45deg, transparent 75%, rgba(127,127,127,0.12) 75%), linear-gradient(-45deg, transparent 75%, rgba(127,127,127,0.12) 75%)",
          backgroundSize: "16px 16px",
          backgroundPosition: "0 0, 0 8px, 8px -8px, -8px 0",
          borderRadius: 1,
        }}
      />
    </Box>
  );
}
