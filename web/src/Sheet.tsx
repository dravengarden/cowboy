import {
  Box,
  Button,
  Dialog,
  DialogContent,
  DialogTitle,
  useMediaQuery,
  useTheme,
} from "@mui/material";
import { type ReactNode, type Ref, useContext, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { BottomSheet, type BottomSheetProps, ScrollEdges } from "@cowboy/app-shell";
import { desktopCompositionActive } from "./desktop/commands/keyIntent";
import {
  desktopDialogSlotProps,
  InstantModalTransition,
  isImeBlockedEscape,
} from "./desktop/DesktopModal";
import { isImeKeyEvent } from "./imeKey";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ObsidianSheet } from "./ObsidianSheet";
import { ENTER_LABEL, hasSendMod, MOD_LABEL } from "./platform";
import { SurfaceContext, useSurfaceProfile } from "./surface/SurfaceProfile";

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

function workbenchPaperSx(
  desktopMaxWidth: number,
  desktopFill: boolean,
  desktopFullScreen: boolean,
): Record<string, unknown> {
  if (desktopFullScreen) {
    return {
      width: "100vw",
      height: "100dvh",
      maxWidth: "none",
      maxHeight: "none",
      m: 0,
      border: 0,
      borderRadius: 0,
    };
  }
  if (desktopFill) {
    return {
      width: "calc(100vw - 32px)",
      height: "calc(100dvh - 32px)",
      maxWidth: "none",
      maxHeight: "none",
      m: 2,
    };
  }
  return {
    width: `min(${desktopMaxWidth}px, calc(100vw - 64px))`,
    maxWidth: "none",
    maxHeight: "calc(100vh - 48px)",
    m: 3,
  };
}

function DesktopTitledSheet({
  open,
  onClose,
  title,
  actions,
  children,
  wide,
  desktopDialogWidth,
  desktopPaperRef,
}: {
  open: boolean;
  onClose: (event: object, reason: string) => void;
  title?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  wide?: boolean | undefined;
  desktopDialogWidth?: number | undefined;
  desktopPaperRef?: Ref<HTMLDivElement> | undefined;
}): React.JSX.Element {
  const titleId = useId();
  return (
    <Dialog
      open={open}
      onClose={onClose}
      aria-labelledby={title != null ? titleId : undefined}
      maxWidth={false}
      fullWidth
      disableScrollLock
      slots={{ transition: InstantModalTransition }}
      slotProps={desktopDialogSlotProps({
        paperRef: desktopPaperRef,
        paperSx: {
          maxWidth: desktopDialogWidth !== undefined
            ? `min(${desktopDialogWidth}px, calc(100vw - 64px))`
            : wide
            ? { sm: 560, lg: 720 }
            : 444,
        },
      })}
    >
      {title == null ? null : (
        <DialogTitle id={titleId} sx={{ fontWeight: 700, pb: 1 }}>
          {title}
        </DialogTitle>
      )}
      <DialogContent>
        {children}
        <ScrollEdges />
      </DialogContent>
      {actions == null ? null : (
        <Box sx={{ display: "flex", justifyContent: "flex-end", gap: 1, px: 3, pb: 2 }}>
          {actions}
        </Box>
      )}
    </Dialog>
  );
}

// cowboy's BottomSheet: always the frosted 磨砂玻璃 material (no per-app toggle —
// the translucent surface is the house look). Drop-in for the shared BottomSheet.
// Compact phone/tablet sheets (no `cover`) use a flush bottom-docked
// action card instead of the tall DetentSheet + floating footer pad.
export function Sheet(
  props: Omit<BottomSheetProps, "frosted"> & {
    desktopMaxWidth?: number | undefined;
    /** Desktop: cap width for titled centered dialogs (default 444, wide 560/720). */
    desktopDialogWidth?: number | undefined;
    /** Optional ref to the Desktop Dialog paper (for topmost-modal guards). */
    desktopPaperRef?: Ref<HTMLDivElement> | undefined;
    /** Desktop: claim the whole viewport less a 16px margin, the Tool
     * inspector's geometry, instead of sizing to the content. */
    desktopFill?: boolean;
    /** Desktop: claim the entire viewport edge to edge (a workbench's own
     * full-screen mode). */
    desktopFullScreen?: boolean;
    portal?: boolean;
  },
): ReactNode {
  const theme = useTheme();
  const widthIsMobile = useMediaQuery(theme.breakpoints.down("sm"));
  // Desktop always keeps modal Dialog surfaces even in narrow (<1200px or
  // <600px) windows; a width-derived `forceSheet={navbarAtBottom}` must never
  // downgrade a Desktop confirmation into a non-modal touch sheet.
  const desktopSurface = useContext(SurfaceContext)?.kind === "desktop";
  const effectiveForceSheet = Boolean(props.forceSheet) && !desktopSurface;
  const effectiveMobile = widthIsMobile && !desktopSurface;
  const {
    desktopMaxWidth,
    desktopDialogWidth,
    desktopPaperRef,
    desktopFill = false,
    desktopFullScreen = false,
    portal = false,
    ...bottomSheetProps
  } = props;
  const handleDesktopClose = (event: object, reason: string): void => {
    if (isImeBlockedEscape(event, reason)) return;
    props.onClose();
  };
  // Titled non-fill Desktop dialogs (filters, details, confirmations) use the
  // scrollable titled dialog so actions stay reachable in short/narrow windows.
  // Untitled or fill/full-screen workbenches (Settings, Calls) own their own
  // internal scroller and bottom rail.
  const useWorkbenchBody = desktopMaxWidth !== undefined && !effectiveMobile &&
    !effectiveForceSheet &&
    !(desktopSurface && props.title != null && !desktopFill && !desktopFullScreen);
  if (useWorkbenchBody) {
    return (
      <Dialog
        open={props.open}
        onClose={handleDesktopClose}
        fullWidth
        maxWidth={false}
        disableScrollLock
        slots={{ transition: InstantModalTransition }}
        slotProps={desktopDialogSlotProps({
          paperRef: desktopPaperRef,
          paperSx: workbenchPaperSx(
            desktopMaxWidth,
            desktopFill,
            desktopFullScreen,
          ),
        })}
      >
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
  if (desktopSurface) {
    return (
      <DesktopTitledSheet
        open={props.open}
        onClose={handleDesktopClose}
        title={props.title}
        actions={props.actions}
        wide={props.wide}
        desktopDialogWidth={desktopDialogWidth ?? desktopMaxWidth}
        desktopPaperRef={desktopPaperRef}
      >
        {props.children}
      </DesktopTitledSheet>
    );
  }
  const useCompactCard = (effectiveForceSheet || effectiveMobile) && !props.cover;
  if (useCompactCard) {
    return maybePortal(
      <ObsidianSheet
        open={props.open}
        onClose={props.onClose}
        title={props.title}
        actions={props.actions}
        ariaLabel={typeof props.title === "string" ? props.title : undefined}
        floatingDismiss={props.mobileDismiss === "footer"}
        fill={props.fill}
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

export interface ConfirmSheetAction {
  readonly label: ReactNode;
  readonly onConfirm: () => void | Promise<void>;
  readonly color?: "primary" | "error" | "warning" | "info" | "secondary";
  readonly disabled?: boolean;
  readonly busy?: boolean;
  readonly cancelLabel?: ReactNode;
  readonly startIcon?: ReactNode;
  readonly confirmProps?: Record<`data-${string}`, string>;
}

export function ConfirmSheet({
  open,
  onClose,
  title,
  children,
  actions: explicitActions,
  confirm,
  wide = false,
  portal = true,
}: {
  open: boolean;
  onClose: () => void;
  title?: ReactNode;
  children: ReactNode;
  actions?: ReactNode;
  /** Explicit opt-in primary action (mutually exclusive with `actions`):
   *  renders Cancel (Esc) + Confirm (Mod+Enter) and binds stack-aware
   *  `useConfirmEnter` from one source of truth. */
  confirm?: ConfirmSheetAction | undefined;
  wide?: boolean;
  /** Folder sheets already live in the keyboard-resized App host. */
  portal?: boolean;
}): ReactNode {
  const forceSheet = useConfirmSheetSurface();
  const dialogRef = useRef<HTMLDivElement>(null);
  const activeConfirm = explicitActions == null ? confirm : undefined;
  const confirmReady = Boolean(
    open && activeConfirm && !activeConfirm.disabled && !activeConfirm.busy,
  );
  const triggerConfirm = (): void => {
    if (!confirmReady || !activeConfirm) return;
    void activeConfirm.onConfirm();
  };
  useConfirmEnter(confirmReady, triggerConfirm, { owner: dialogRef });
  const actions = explicitActions ?? (activeConfirm
    ? (
      <>
        <Button
          color="inherit"
          disabled={Boolean(activeConfirm.busy)}
          onClick={onClose}
          sx={{ textTransform: "none" }}
        >
          {activeConfirm.cancelLabel ?? "Cancel"}
          {!forceSheet && <Kbd keys="Esc" />}
        </Button>
        <Button
          {...activeConfirm.confirmProps}
          variant="contained"
          color={activeConfirm.color ?? "primary"}
          disabled={!confirmReady}
          startIcon={activeConfirm.startIcon}
          onKeyDown={(e): void => {
            // Block native bare/repeat/Shift+Enter activation on the
            // affirmative button on Desktop so holding Enter or pressing
            // Shift+Enter can never confirm an irreversible action.
            if (
              !forceSheet && e.key === "Enter" && !hasSendMod(e.nativeEvent) &&
              !isImeKeyEvent(e.nativeEvent) && !desktopCompositionActive()
            ) {
              e.preventDefault();
            }
          }}
          onClick={triggerConfirm}
          sx={{ textTransform: "none" }}
        >
          {activeConfirm.label}
          {!forceSheet && (
            <Kbd
              keys={`${MOD_LABEL}${ENTER_LABEL}`}
              availability={confirmReady ? "available" : "inactive"}
            />
          )}
        </Button>
      </>
    )
    : undefined);
  return (
    <Sheet
      open={open}
      onClose={onClose}
      title={title}
      actions={actions}
      wide={wide}
      portal={portal}
      desktopPaperRef={dialogRef}
      forceSheet={forceSheet}
      mobileDismiss={actions == null ? "footer" : "none"}
    >
      {children}
    </Sheet>
  );
}
