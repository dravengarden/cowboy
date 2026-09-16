import { InsertDriveFileOutlined, PictureAsPdfOutlined } from "@mui/icons-material";
import { Box, Stack, Typography } from "@mui/material";

const MIME_LABELS: Record<string, string> = {
  "application/pdf": "PDF",
  "application/json": "JSON",
  "application/zip": "ZIP",
  "text/plain": "Text",
  "text/markdown": "Markdown",
  "text/csv": "CSV",
};

/** Short type label for a sent file: the extension when the name has one,
 *  otherwise a known MIME type, otherwise the generic "File". */
export function attachmentKindLabel(name: string, mimeType?: string): string {
  const ext = /\.([A-Za-z0-9]{1,8})$/u.exec(name)?.[1];
  if (ext) return ext.toUpperCase();
  const mime = mimeType?.split(";")[0]?.trim().toLowerCase();
  return (mime && MIME_LABELS[mime]) ?? "File";
}

/** A non-image attachment inside a message bubble. Images get a thumbnail;
 *  a file has no preview, but the reader must still see that the message
 *  carried it — as a card on its own row, not a paperclip glued to the prose. */
export function TranscriptFileChip({
  name,
  mimeType,
  invert,
}: {
  name: string;
  mimeType?: string | undefined;
  /** Rendered on the primary-filled user bubble. */
  invert: boolean;
}): React.JSX.Element {
  const kind = attachmentKindLabel(name, mimeType);
  const Icon = kind === "PDF" ? PictureAsPdfOutlined : InsertDriveFileOutlined;
  return (
    <Stack
      data-transcript-file-attachment="true"
      direction="row"
      spacing={1}
      alignItems="center"
      title={name}
      sx={{
        width: "fit-content",
        maxWidth: "min(360px, 100%)",
        my: 0.5,
        py: 0.75,
        pl: 1,
        pr: 1.5,
        borderRadius: 1,
        border: 1,
        borderColor: invert ? "rgba(255,255,255,0.28)" : "divider",
        bgcolor: invert ? "rgba(255,255,255,0.14)" : "action.hover",
        color: invert ? "inherit" : "text.primary",
      }}
    >
      <Box
        aria-hidden
        sx={{
          display: "grid",
          placeItems: "center",
          width: 32,
          height: 32,
          flexShrink: 0,
          borderRadius: 0.75,
          bgcolor: invert ? "rgba(255,255,255,0.18)" : "background.paper",
        }}
      >
        <Icon fontSize="small" />
      </Box>
      <Box sx={{ minWidth: 0 }}>
        <Typography variant="body2" noWrap sx={{ fontWeight: 600 }}>
          {name}
        </Typography>
        <Typography
          variant="caption"
          noWrap
          sx={{ display: "block", opacity: 0.72, color: "inherit" }}
        >
          {kind}
        </Typography>
      </Box>
    </Stack>
  );
}
