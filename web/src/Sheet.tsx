import { Dialog, DialogContent, useMediaQuery, useTheme } from "@mui/material";
import type { ReactNode } from "react";
import { BottomSheet, type BottomSheetProps } from "./_shell";

// cowboy's BottomSheet: always the frosted 磨砂玻璃 material (no per-app toggle —
// the translucent surface is the house look). Drop-in for the shared BottomSheet.
export function Sheet(
  props: Omit<BottomSheetProps, "frosted"> & { desktopMaxWidth?: number },
): ReactNode {
  const theme = useTheme();
  const widthIsMobile = useMediaQuery(theme.breakpoints.down("sm"));
  const { desktopMaxWidth, ...bottomSheetProps } = props;
  // Content-heavy Cowboy workbenches may claim the available Desktop canvas.
  // Mobile and touch-forced sheets keep the shared DetentSheet unchanged.
  if (
    desktopMaxWidth !== undefined && !widthIsMobile && !props.forceSheet
  ) {
    return (
      <Dialog
        open={props.open}
        onClose={props.onClose}
        fullWidth
        maxWidth={false}
        slotProps={{
          paper: {
            sx: {
              width: `min(${desktopMaxWidth}px, calc(100vw - 64px))`,
              maxWidth: "none",
              maxHeight: "calc(100vh - 48px)",
              m: 3,
            },
          },
        }}
      >
        <DialogContent>{props.children}</DialogContent>
      </Dialog>
    );
  }
  return <BottomSheet mobileDismiss="footer" floatingActions {...bottomSheetProps} frosted />;
}
