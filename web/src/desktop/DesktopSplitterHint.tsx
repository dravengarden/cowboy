import { Box, Stack, Typography } from "@mui/material";
import { ShortcutKeycap } from "../ShortcutKeycap";

export function DesktopSplitterHint(): React.JSX.Element {
  return (
    <Stack
      data-desktop-splitter-hint
      direction="row"
      alignItems="center"
      spacing={0.45}
      sx={{
        position: "absolute",
        top: "50%",
        left: "50%",
        transform: "translate(-50%, -50%)",
        px: 0.65,
        py: 0.4,
        border: 1,
        borderColor: "primary.main",
        borderRadius: 1.25,
        bgcolor: "background.paper",
        boxShadow: 3,
        color: "primary.main",
        pointerEvents: "none",
        whiteSpace: "nowrap",
        zIndex: 5,
      }}
    >
      <ShortcutKeycap keyLabel="H" variant="context" accent availability="active" />
      <Typography variant="caption" fontWeight={750} sx={{ lineHeight: 1 }}>
        Resize
      </Typography>
      <ShortcutKeycap keyLabel="L" variant="context" accent availability="active" />
      <Box
        component="span"
        sx={{ color: "text.secondary", fontSize: "0.625rem", ml: "2px !important" }}
      >
        · Tab
      </Box>
    </Stack>
  );
}
