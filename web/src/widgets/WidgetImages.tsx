// `image` and `gallery` nodes (docs/widgets.md §5.4, §7.3). Only a published
// widget draws pixels: the Controller verified each `artifact` source and
// resolved every `event` source to one, and decoded each `data` PNG. Those
// load through `ProtectedImage` (authenticated, never a remote address) and
// open in the product lightbox. An inline fence, or any source this client
// cannot verify, shows the `alt` text instead.

import { lazy, Suspense } from "react";
import { Box, Typography } from "@mui/material";
import ImageOutlined from "@mui/icons-material/ImageOutlined";
import { WIDGET_LIMITS, type ImageSource } from "../../../contracts/widget-v1.ts";
import { moreLabel, previewSlice, useWidgetMode, useWidgetVerified } from "./widgetMode";
import type { ParsedNode } from "./widgetDocument";

type Of<T extends ParsedNode["type"]> = Extract<ParsedNode, { type: T }>;
export type Picture = { src: ImageSource; alt: string; caption?: string | undefined };

const ARTIFACT = /^\/api\/artifacts\/[0-9a-f]{64}\.(?:png|jpg|webp|gif|avif)$/;
const HEIGHT = { xs: 80, sm: 160, md: 280, lg: 420 } as const;
const DrawnPictures = lazy(() => import("./WidgetPictures"));

/** The URL a verified source loads from, or null when only `alt` may show. */
export function pictureUrl(src: ImageSource, verified: boolean): string | null {
  if (!verified) return null;
  if (src.kind === "artifact") return ARTIFACT.test(src.url) ? src.url : null;
  if (src.kind === "data" && src.mime === "image/png" && /^[A-Za-z0-9+/]+=*$/.test(src.base64)) {
    return `data:image/png;base64,${src.base64}`;
  }
  return null;
}

export function ImagePlaceholder({
  alt,
  caption,
}: {
  alt: string;
  caption?: string | undefined;
}): React.JSX.Element {
  return (
    <Box
      role="img"
      aria-label={alt}
      data-widget-image-placeholder=""
      sx={{
        display: "flex",
        gap: 1,
        alignItems: "center",
        border: 1,
        borderColor: "divider",
        borderRadius: 1,
        px: 1.5,
        py: 1,
        color: "text.secondary",
      }}
    >
      <ImageOutlined fontSize="small" />
      <Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>
        {alt}
        {caption ? ` — ${caption}` : ""}
      </Typography>
    </Box>
  );
}

function Pictures(props: {
  pictures: readonly Picture[];
  height: number;
  fit: "contain" | "cover";
  grid: boolean;
}): React.JSX.Element {
  const verified = useWidgetVerified();
  const urls = props.pictures.map((picture) => pictureUrl(picture.src, verified));
  const placeholders = (
    <Box
      sx={
        props.grid
          ? {
              display: "grid",
              gridTemplateColumns: "repeat(auto-fill, minmax(min(200px, 100%), 1fr))",
              gap: 1,
            }
          : { minWidth: 0 }
      }
    >
      {props.pictures.map((picture, i) => (
        <ImagePlaceholder key={i} alt={picture.alt} caption={picture.caption} />
      ))}
    </Box>
  );
  if (urls.every((url) => url === null)) return placeholders;
  return (
    <Suspense fallback={placeholders}>
      <DrawnPictures {...props} urls={urls} />
    </Suspense>
  );
}

export function ImageNode({ node }: { node: Of<"image"> }): React.JSX.Element {
  return (
    <Pictures
      pictures={[{ src: node.src, alt: node.alt, caption: node.caption }]}
      height={HEIGHT[node.height ?? "md"]}
      fit={node.fit ?? "contain"}
      grid={false}
    />
  );
}

export function GalleryNode({ node }: { node: Of<"gallery"> }): React.JSX.Element {
  const { shown, hidden } = previewSlice(node.images, WIDGET_LIMITS.previewImages, useWidgetMode());
  return (
    <Box>
      <Pictures pictures={shown} height={HEIGHT.sm} fit="cover" grid />
      {hidden > 0 ? (
        <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.5 }}>
          {moreLabel(hidden, "image")}
        </Typography>
      ) : null}
    </Box>
  );
}
