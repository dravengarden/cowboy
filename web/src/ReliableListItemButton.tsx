import { type ComponentPropsWithoutRef, forwardRef } from "react";
import { alpha, ListItemButton } from "@mui/material";
import { useReliableTouchTap } from "./useReliableTouchTap";

export const ReliableListItemButton = forwardRef<
  HTMLDivElement,
  Omit<ComponentPropsWithoutRef<typeof ListItemButton>, "onClick"> & {
    onActivate: () => void;
  }
>(function ReliableListItemButton(
  {
    onActivate,
    sx,
    onPointerDownCapture,
    onPointerDown,
    onPointerEnter,
    onKeyDown,
    ...props
  },
  ref,
) {
  const tap = useReliableTouchTap<HTMLDivElement>(onActivate);
  return (
    <ListItemButton
      {...props}
      onPointerDownCapture={(event): void => {
        // A grip touch stops propagation before the row's bubble
        // handler. Mark touch in capture so iOS cannot leave the
        // ancestor ListItemButton's synthetic :hover latched.
        if (event.pointerType === "touch") {
          event.currentTarget.dataset.touchActivated = "true";
        } else if (event.pointerType === "mouse") {
          delete event.currentTarget.dataset.touchActivated;
        }
        onPointerDownCapture?.(event);
      }}
      onPointerDown={(event): void => {
        onPointerDown?.(event);
        tap.onPointerDown(event);
      }}
      onPointerEnter={(event): void => {
        if (event.pointerType === "mouse") {
          delete event.currentTarget.dataset.touchActivated;
        }
        onPointerEnter?.(event);
      }}
      onKeyDown={(event): void => {
        delete event.currentTarget.dataset.touchActivated;
        onKeyDown?.(event);
      }}
      onPointerMove={tap.onPointerMove}
      onPointerUp={tap.onPointerUp}
      onPointerCancel={tap.onPointerCancel}
      onClick={tap.onClick}
      ref={ref}
      sx={[
        {
          touchAction: "manipulation",
          // iOS WebKit synthesizes and latches :hover after a finger
          // touch. Keep real mouse/keyboard feedback, but let touch
          // rows return to their normal selected/unselected material.
          "&[data-touch-activated='true']:not(.Mui-selected):hover, &[data-touch-activated='true'].Mui-focusVisible:not(.Mui-selected)":
            {
              bgcolor: "transparent",
            },
          "&[data-touch-activated='true'].Mui-selected:hover, &[data-touch-activated='true'].Mui-selected.Mui-focusVisible":
            {
              bgcolor: (theme) =>
                alpha(
                  theme.palette.primary.main,
                  theme.palette.action.selectedOpacity,
                ),
            },
          "&[data-touch-activated='true']:active": {
            bgcolor: "action.selected",
          },
        },
        ...(Array.isArray(sx) ? sx : [sx]),
      ]}
    />
  );
});
