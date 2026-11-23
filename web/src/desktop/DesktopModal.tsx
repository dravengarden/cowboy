import { ScrollEdges } from "@cowboy/app-shell";
import {
  Box,
  Dialog,
  Divider,
  IconButton,
  Stack,
  Typography,
} from "@mui/material";
import { Close } from "@mui/icons-material";
import {
  cloneElement,
  forwardRef,
  type KeyboardEventHandler,
  type ReactElement,
  type ReactNode,
  type Ref,
  useCallback,
  useEffect,
} from "react";
import { isImeKeyEvent } from "../imeKey";
import { HintTooltip } from "../HintTooltip";
import {
  desktopModalBackdropSx,
  desktopModalPaperSx,
} from "./DesktopEmbeddedControl";
import {
  DesktopShortcutBar,
  type DesktopShortcutGroup,
} from "./DesktopShortcutBar";
import {
  desktopModalShortcutGroups,
  useDesktopModalMode,
} from "./DesktopModalKeyHint";

function assignRef<T>(target: Ref<T> | undefined, value: T | null): void {
  if (typeof target === "function") target(value);
  else if (target && typeof target === "object") {
    (target as { current: T | null }).current = value;
  }
}

/** Desktop dialogs open immediately without MUI Fade's synchronous
 *  `node.scrollTop` reflow on mount. */
const InstantModalTransition = forwardRef<
  HTMLElement,
  {
    in?: boolean;
    children: ReactElement<{ ref?: Ref<HTMLElement> }>;
    onEnter?: () => void;
    onEntered?: () => void;
    onExit?: () => void;
    onExited?: () => void;
  }
>(function InstantModalTransition(
  { in: open, children, onEnter, onEntered, onExit, onExited },
  ref,
) {
  const childRef = children.props.ref;
  const mergedRef = useCallback(
    (node: HTMLElement | null): void => {
      assignRef(ref, node);
      assignRef(childRef, node);
    },
    [ref, childRef],
  );
  useEffect(() => {
    if (open) {
      onEnter?.();
      onEntered?.();
    } else {
      onExit?.();
      onExited?.();
    }
  }, [open, onEnter, onEntered, onExit, onExited]);
  return open ? cloneElement(children, { ref: mergedRef }) : null;
});

export function DesktopModal({
  open,
  onClose,
  title,
  description,
  icon,
  children,
  footer,
  shortcutGroups,
  onShortcutKeyDown,
  width = 920,
  fill = false,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: string;
  icon?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  shortcutGroups?: readonly DesktopShortcutGroup[];
  onShortcutKeyDown?: KeyboardEventHandler<HTMLDivElement>;
  width?: number;
  /** A workbench rather than a form: take the whole viewport less a margin
   *  and let the content lay out its own scrolling regions. */
  fill?: boolean;
}): React.JSX.Element | null {
  if (!open) return null;
  return (
    <Dialog
      open={open}
      onClose={(event, reason): void => {
        if (reason === "escapeKeyDown") {
          const keyEvent = "nativeEvent" in event
            ? event.nativeEvent as KeyboardEvent
            : event as KeyboardEvent;
          if (isImeKeyEvent(keyEvent)) return;
        }
        onClose();
      }}
      fullWidth
      maxWidth={false}
      disableScrollLock
      slots={{ transition: InstantModalTransition }}
      slotProps={{
        ...(onShortcutKeyDown
          ? { root: { onKeyDown: onShortcutKeyDown } }
          : {}),
        paper: {
          sx: {
            ...desktopModalPaperSx(),
            ...(fill
              ? {
                width: "min(1760px, calc(100vw - 48px))",
                height: "calc(100vh - 48px)",
                maxWidth: "none",
                maxHeight: "none",
                m: 3,
              }
              : {
                width: `min(${width}px, calc(100vw - 64px))`,
                maxHeight: "min(860px, calc(100vh - 64px))",
                m: 4,
              }),
          },
        },
        backdrop: {
          slots: { transition: InstantModalTransition },
          sx: desktopModalBackdropSx(),
        },
      }}
    >
      <Stack direction="row" alignItems="center" spacing={1.25} sx={{ px: 2.25, py: 1.55 }}>
        {icon}
        <Box sx={{ minWidth: 0 }}>
          <Typography variant="subtitle1" fontWeight={780}>{title}</Typography>
          {description && <Typography variant="caption" color="text.secondary">{description}</Typography>}
        </Box>
        <Box sx={{ flex: 1 }} />
        <HintTooltip title="Close" shortcut="Escape">
          <IconButton aria-label={`Close ${title}`} onClick={onClose} size="small">
            <Close fontSize="small" />
          </IconButton>
        </HintTooltip>
      </Stack>
      <Divider />
      <Box
        sx={fill
          ? { flex: 1, minHeight: 0, display: "flex", overflow: "hidden" }
          : { minHeight: 0, overflow: "auto" }}
      >
        {children}
      {!fill && <ScrollEdges />}</Box>
      {footer && <><Divider />{footer}</>}
      {shortcutGroups
        ? <DesktopShortcutBar groups={shortcutGroups} />
        : <DesktopModalGrammarBar />}
    </Dialog>
  );
}

/** A dialog without its own keymap advertises the shared modal grammar. */
function DesktopModalGrammarBar(): React.JSX.Element {
  const mode = useDesktopModalMode();
  return <DesktopShortcutBar groups={desktopModalShortcutGroups(mode)} />;
}
