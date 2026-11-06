import { MoreHoriz, SettingsOutlined } from "@mui/icons-material";
import { Box, Button, IconButton, Stack } from "@mui/material";
import { type ReactNode, useState } from "react";
import { openAppSettings } from "../../appSettings";
import { Sheet } from "../../Sheet";
import { useSurfaceProfile } from "../../surface/SurfaceProfile";

/** Secondary tools keep their place without taking space from the reader. */
export function ReviewMoreActions(
  { children }: { children: ReactNode },
): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const surface = useSurfaceProfile();
  if (surface.kind === "desktop") {
    return (
      <Stack direction="row" alignItems="center" sx={{ flexWrap: "wrap" }}>
        {children}
        <IconButton
          aria-label="Review settings"
          onClick={() => openAppSettings({ section: "code" })}
        >
          <SettingsOutlined />
        </IconButton>
      </Stack>
    );
  }
  return (
    <>
      <IconButton
        aria-label="More review actions"
        aria-haspopup="dialog"
        onClick={() => setOpen(true)}
      >
        <MoreHoriz />
      </IconButton>
      <Sheet
        open={open}
        onClose={() => setOpen(false)}
        title="Review actions"
        forceSheet
        portal
        actions={<Button aria-label="Close review actions" onClick={() => setOpen(false)} sx={{ minHeight: 44 }}>Done</Button>}
      >
        <Stack spacing={1} sx={{ px: 1, pb: 1 }}>
          <Box
            sx={{
              display: "flex",
              flexWrap: "wrap",
              alignItems: "center",
              gap: 1,
            }}
          >
            {children}
          </Box>
          <Button
            startIcon={<SettingsOutlined />}
            color="inherit"
            sx={{ minHeight: 44, justifyContent: "flex-start" }}
            onClick={() => {
              setOpen(false);
              openAppSettings({ section: "code" });
            }}
          >
            Review settings
          </Button>
        </Stack>
      </Sheet>
    </>
  );
}
