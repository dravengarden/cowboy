import { Dialog, DialogContent, useMediaQuery, useTheme } from "@mui/material";
import type { ReactNode } from "react";
import { createPortal } from "react-dom";
import { BottomSheet, type BottomSheetProps } from "./_shell";

// cowboy's BottomSheet: always the frosted 磨砂玻璃 material (no per-app toggle —
// the translucent surface is the house look). Drop-in for the shared BottomSheet.
export function Sheet(
  props: Omit<BottomSheetProps, "frosted"> & {
    desktopMaxWidth?: number;
    portal?: boolean;
  },
): ReactNode {
  const theme = useTheme();
  const widthIsMobile = useMediaQuery(theme.breakpoints.down("sm"));
  const { desktopMaxWidth, portal = false, ...bottomSheetProps } = props;
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
        {
          /* Desktop workbenches own a fixed command rail as their final child.
            The workbench's middle child scrolls; keeping the DialogContent
            itself out of the scroll chain makes the rail visible from the
            moment the modal opens. */
        }
        <DialogContent
          sx={{
            pb: 0,
            minHeight: 0,
            overflow: "hidden",
            display: "flex",
            flexDirection: "column",
          }}
        >
          {props.children}
        </DialogContent>
      </Dialog>
    );
  }
  const sheet = (
    <BottomSheet
      mobileDismiss="footer"
      floatingActions
      {...bottomSheetProps}
      frosted
    />
  );
  // A picker nested inside a full-screen sheet otherwise inherits iOS
  // WebKit's momentum-scroller containing block. Its fixed scrim then starts
  // below the parent's safe-area/drag header instead of at the viewport edge.
  // Cowboy owns the top document, so nested workbench pickers may explicitly
  // escape that inline SDK boundary while ordinary/hosted sheets stay inline.
  return portal && globalThis.document?.body
    ? createPortal(sheet, globalThis.document.body)
    : sheet;
}
