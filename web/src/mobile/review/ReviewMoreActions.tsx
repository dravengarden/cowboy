import { MoreHoriz, SettingsOutlined } from "@mui/icons-material";
import { Box, Button, IconButton, Stack } from "@mui/material";
import { type ReactNode, useCallback, useState } from "react";
import { openAppSettings } from "../../appSettings";
import { Sheet } from "../../Sheet";
import { useSurfaceProfile } from "../../surface/SurfaceProfile";

const DISMISS_ACTIONS_EVENT = "cowboy:review-dismiss-actions";

/** Portal actions keep their React owner, so notify the DOM action host only
 * after their handler has opened the destination. */
export function dismissReviewActions(container: HTMLElement): void {
  container.dispatchEvent(new Event(DISMISS_ACTIONS_EVENT, { bubbles: true }));
}

/** Secondary tools keep their place without taking space from the reader. */
export function ReviewMoreActions({
  children,
}: {
  children: (close: () => void) => ReactNode;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const content = useCallback((node: HTMLDivElement | null) => {
    if (!node) return;
    const dismiss = () => setOpen(false);
    node.addEventListener(DISMISS_ACTIONS_EVENT, dismiss);
    return () => node.removeEventListener(DISMISS_ACTIONS_EVENT, dismiss);
  }, []);
  const surface = useSurfaceProfile();
  const close = () => setOpen(false);
  if (surface.kind === "desktop") {
    return (
      <Stack direction="row" alignItems="center" sx={{ flexWrap: "wrap" }}>
        {children(close)}
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
        actions={
          <Button
            aria-label="Close review actions"
            onClick={() => setOpen(false)}
            sx={{ minHeight: 44 }}
          >
            Done
          </Button>
        }
      >
        <Stack spacing={1} sx={{ px: 1, pb: 1 }}>
          <Box
            ref={content}
            // One action per row, aligned like Review settings below, so a
            // short label never pairs up with its neighbour.
            sx={{
              display: "flex",
              flexDirection: "column",
              alignItems: "stretch",
              gap: 1,
              // Inline action groups are horizontal on Desktop; stack them here.
              "& [data-review-navigation-actions]": {
                flexDirection: "column",
                alignItems: "stretch",
                gap: 1,
              },
              "& [data-review-navigation-actions] > span": { display: "flex" },
              "& .MuiButton-root": {
                flex: 1,
                minHeight: 44,
                justifyContent: "flex-start",
                textTransform: "none",
              },
              "& .MuiButton-root:not(.Mui-disabled)": {
                color: "text.primary",
              },
            }}
          >
            {children(close)}
          </Box>
          <Button
            startIcon={<SettingsOutlined />}
            color="inherit"
            sx={{ minHeight: 44, justifyContent: "flex-start", textTransform: "none" }}
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
