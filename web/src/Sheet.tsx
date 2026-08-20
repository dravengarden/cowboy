import { Dialog, DialogContent, useMediaQuery, useTheme } from "@mui/material";
import type { ReactNode } from "react";
import { createPortal } from "react-dom";
import { BottomSheet, type BottomSheetProps } from "./_shell";
import { MobileDecisionDock } from "./MobileThumbDock";
import { ObsidianSheet } from "./ObsidianSheet";
import { useSurfaceProfile } from "./surface/SurfaceProfile";

function maybePortal(sheet: ReactNode, portal: boolean): ReactNode {
  // A picker nested inside a full-screen sheet otherwise inherits iOS
  // WebKit's momentum-scroller containing block. Its fixed scrim then starts
  // below the parent's safe-area/drag header instead of at the viewport edge.
  // Cowboy owns the top document, so nested workbench pickers may explicitly
  // escape that inline SDK boundary while ordinary/hosted sheets stay inline.
  return portal && globalThis.document?.body
    ? createPortal(sheet, globalThis.document.body)
    : sheet;
}

// cowboy's BottomSheet: always the frosted 磨砂玻璃 material (no per-app toggle —
// the translucent surface is the house look). Drop-in for the shared BottomSheet.
// Compact phone/tablet sheets (no `cover`) use a flush bottom-docked
// action card instead of the tall DetentSheet + floating footer pad.
export function Sheet(
  props: Omit<BottomSheetProps, "frosted"> & {
    desktopMaxWidth?: number;
    portal?: boolean;
    dockClearance?: boolean;
  },
): ReactNode {
  const theme = useTheme();
  const widthIsMobile = useMediaQuery(theme.breakpoints.down("sm"));
  const {
    desktopMaxWidth,
    portal = false,
    dockClearance = false,
    ...bottomSheetProps
  } = props;
  // Content-heavy Cowboy workbenches may claim the available Desktop canvas.
  // Cover sheets stay on DetentSheet; compact phone/tablet sheets use the
  // flush bottom card.
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
  const useCompactCard = (props.forceSheet || widthIsMobile) && !props.cover;
  if (useCompactCard) {
    return maybePortal(
      <ObsidianSheet
        open={props.open}
        onClose={props.onClose}
        title={props.title}
        actions={props.actions}
        dockClearance={dockClearance}
        ariaLabel={typeof props.title === "string" ? props.title : undefined}
      >
        {props.children}
      </ObsidianSheet>,
      portal,
    );
  }
  return maybePortal(
    <BottomSheet
      mobileDismiss="footer"
      floatingActions
      {...bottomSheetProps}
      frosted
    />,
    portal,
  );
}

/**
 * Confirmation and other compact decision modals follow product identity.
 * Mobile and tablet always rise as the Obsidian-style inset card — including
 * iPhone landscape. Desktop keeps the shared centered dialog. Do not open a
 * raw MUI Dialog for these prompts: a phone-width surface is a bottom card,
 * not a floating centered dialog.
 */
export function useConfirmSheetSurface(): boolean {
  return useSurfaceProfile().kind !== "desktop";
}

export interface ConfirmSheetDock {
  readonly cancelLabel?: string | undefined;
  readonly cancelDisabled?: boolean | undefined;
  readonly confirmLabel: string;
  readonly confirmDisabled?: boolean | undefined;
  readonly confirmBusy?: boolean | undefined;
  readonly onConfirm: () => void;
}

export function ConfirmSheet({
  open,
  onClose,
  title,
  children,
  actions,
  dock,
  wide = false,
}: {
  open: boolean;
  onClose: () => void;
  title?: ReactNode;
  children: ReactNode;
  actions?: ReactNode | undefined;
  dock?: ConfirmSheetDock | undefined;
  wide?: boolean;
}): ReactNode {
  const forceSheet = useConfirmSheetSurface();
  const useDock = forceSheet && dock != null;
  return (
    <>
      <Sheet
        open={open}
        onClose={onClose}
        title={title}
        actions={useDock ? undefined : actions}
        dockClearance={useDock}
        wide={wide}
        portal
        forceSheet={forceSheet}
        mobileDismiss={useDock || actions != null ? "none" : "footer"}
      >
        {children}
      </Sheet>
      {dock == null ? null : (
        <MobileDecisionDock
          open={open && useDock}
          cancelLabel={dock.cancelLabel}
          onCancel={onClose}
          cancelDisabled={dock.cancelDisabled}
          confirmLabel={dock.confirmLabel}
          onConfirm={dock.onConfirm}
          confirmDisabled={dock.confirmDisabled}
          confirmBusy={dock.confirmBusy}
        />
      )}
    </>
  );
}
