// Draws verified widget images (lazy: only widgets with pixels load it).
// `ProtectedImage` fetches same-origin artifacts with the session cookie; the
// product lightbox pages the images of one node.

import { useState } from "react";
import { Box, ButtonBase, Typography } from "@mui/material";
import { ProtectedImage } from "../ProtectedImage";
import { ProtectedImageLightbox } from "../ProtectedImageLightbox";
import { ImagePlaceholder, type Picture } from "./WidgetImages";

/** A Controller-verified `data` image is at most 256 px; draw it in ≤ 128 px. */
const DATA_BOX = 128;

function Figure({
  picture,
  url,
  height,
  fit,
  onOpen,
}: {
  picture: Picture;
  url: string;
  height: number;
  fit: "contain" | "cover";
  onOpen: () => void;
}): React.JSX.Element {
  const data = picture.src.kind === "data";
  return (
    <Box component="figure" sx={{ m: 0, minWidth: 0, display: "grid", gap: 0.5 }}>
      <ButtonBase
        aria-label={`Open image: ${picture.alt}`}
        data-widget-image=""
        onClick={onOpen}
        sx={{
          justifySelf: fit === "cover" ? "stretch" : "start",
          maxWidth: "100%",
          // A glyph-sized data image keeps its corners; photos take the card radius.
          borderRadius: data ? "4px" : 1,
          overflow: "hidden",
          border: 1,
          borderColor: "divider",
          bgcolor: "action.hover",
          cursor: "zoom-in",
        }}
      >
        <ProtectedImage
          src={url}
          alt={picture.alt}
          loading="lazy"
          style={
            data
              ? { display: "block", maxWidth: DATA_BOX, maxHeight: DATA_BOX }
              : fit === "cover"
                ? { display: "block", width: "100%", height, objectFit: "cover" }
                : { display: "block", maxWidth: "100%", maxHeight: height, objectFit: "contain" }
          }
        />
      </ButtonBase>
      {picture.caption ? (
        <Typography component="figcaption" variant="caption" color="text.secondary">
          {picture.caption}
        </Typography>
      ) : null}
    </Box>
  );
}

export default function Pictures({
  pictures,
  height,
  fit,
  grid,
  urls,
}: {
  pictures: readonly Picture[];
  height: number;
  fit: "contain" | "cover";
  grid: boolean;
  urls: readonly (string | null)[];
}): React.JSX.Element {
  const [index, setIndex] = useState<number | null>(null);
  // The lightbox pages only the images that loaded, in reading order.
  const zoomable = pictures.flatMap((picture, i) => {
    const src = urls[i];
    return src ? [{ src, alt: picture.alt, at: i }] : [];
  });
  return (
    <Box
      sx={
        grid
          ? {
              display: "grid",
              gridTemplateColumns: "repeat(auto-fill, minmax(min(200px, 100%), 1fr))",
              gap: 1,
            }
          : { minWidth: 0 }
      }
    >
      {pictures.map((picture, i) => {
        const url = urls[i];
        return url ? (
          <Figure
            key={i}
            picture={picture}
            url={url}
            height={height}
            fit={fit}
            onOpen={(): void => setIndex(zoomable.findIndex((entry) => entry.at === i))}
          />
        ) : (
          <ImagePlaceholder key={i} alt={picture.alt} caption={picture.caption} />
        );
      })}
      {zoomable.length > 0 ? (
        <ProtectedImageLightbox
          images={zoomable.map(({ src, alt }) => ({ src, alt }))}
          index={index}
          onIndex={setIndex}
          onClose={(): void => setIndex(null)}
        />
      ) : null}
    </Box>
  );
}

