import { memo } from "react";
import { Box, Stack, Typography } from "@mui/material";
import { InsertDriveFileOutlined } from "@mui/icons-material";
import type { Attachment } from "./attachments";
import { ProtectedImage } from "./ProtectedImage";
import { openLightbox } from "./ResourceLightbox";
import { glyphSize } from "./surface/desktopSize";

// Read-only attachment preview for a parked draft / queued row: actual mini
// thumbnails (images) + name chips (files), each tapping into the full-screen
// lightbox. A parked message now SHOWS what it carries instead of a blind
// "2 attachments" count. Removal is via editing the row, so no × here.
export const QueuedAttachmentChips = memo(function QueuedAttachmentChips({
  attachments,
}: {
  attachments: Attachment[];
}): React.JSX.Element {
  return (
    <Box
      sx={{ display: "inline-flex", flexWrap: "wrap", gap: 0.5, mt: 0.5 }}
    >
      {attachments.map((a, i) =>
        a.isImage && a.previewUrl
          ? (
            <Box
              key={a.id}
              component={ProtectedImage}
              src={a.previewUrl}
              alt={a.name}
              data-pending-content-action="attachment-preview"
              onClick={(event): void => {
                event.stopPropagation();
                openLightbox(attachments, i);
              }}
              sx={{
                width: 38,
                height: 38,
                objectFit: "cover",
                borderRadius: 0.75,
                border: 1,
                borderColor: "divider",
                cursor: "pointer",
                display: "block",
              }}
            />
          )
          : (
            <Stack
              key={a.id}
              direction="row"
              spacing={0.5}
              alignItems="center"
              data-pending-content-action="attachment-preview"
              onClick={(event): void => {
                event.stopPropagation();
                openLightbox(attachments, i);
              }}
              sx={{
                height: 38,
                maxWidth: 150,
                px: 0.75,
                borderRadius: 0.75,
                border: 1,
                borderColor: "divider",
                bgcolor: "action.hover",
                color: "text.secondary",
                cursor: "pointer",
              }}
            >
              <InsertDriveFileOutlined sx={{ fontSize: glyphSize(15), flexShrink: 0 }} />
              <Typography variant="caption" noWrap sx={{ minWidth: 0 }}>
                {a.name}
              </Typography>
            </Stack>
          )
      )}
    </Box>
  );
}, (a, b) =>
  a.attachments.length === b.attachments.length &&
  a.attachments.every((x, i) => x.id === b.attachments[i]?.id));
