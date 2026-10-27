import {
  type KeyboardEvent as ReactKeyboardEvent,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  Box,
  ListItemIcon,
  ListItemText,
  Menu,
  MenuItem,
  type PopoverActions,
  type Theme,
} from "@mui/material";
import {
  AltRoute,
  FirstPage,
  PauseCircleOutline,
  Schedule,
  SkipNext,
} from "@mui/icons-material";
import type { DeliveryTarget } from "./currentTurnDelivery";
import { type DeliveryOptions, readDeliveryOptions } from "./deliveryPolicy";
import {
  handleDeliveryPickerKey,
  registerDeliveryPickerCommands,
} from "./deliveryPickerCommands";
import { DesktopShortcut } from "./desktop/commands/DesktopKeycap";
import { useSurfaceProfile } from "./surface/SurfaceProfile";

export interface DeliveryPickerProps {
  sessionId: string;
  anchor: HTMLElement | null;
  busy: boolean;
  paused: boolean;
  queueCount: number;
  onClose: () => void;
  onNextTurn: () => void;
  onQueueFront: () => void;
  onSteer: (target: DeliveryTarget, nextCheckpoint?: boolean) => void;
  onSendAndPause: () => void;
  onStop: () => void;
}

export function DeliveryPicker(props: DeliveryPickerProps): React.JSX.Element {
  const desktop = useSurfaceProfile().kind === "desktop";
  const [options, setOptions] = useState<DeliveryOptions | null>(null);
  const [error, setError] = useState<string | null>(null);
  const list = useRef<HTMLUListElement>(null);
  const position = useRef<PopoverActions>(null);
  const interacted = useRef(false);
  const initializedFocus = useRef(false);
  const open = props.anchor !== null;
  const anchorRect = useRef<DOMRect | null>(null);
  // Keyboard dismissal and busy/idle transitions can replace the real button.
  // Retain the last visible rect instead of letting Popover fall back to body.
  if (props.anchor?.isConnected) {
    const rect = props.anchor.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) anchorRect.current = rect;
  }
  const virtualAnchor = useRef({
    nodeType: 1 as const,
    getBoundingClientRect: (): DOMRect => anchorRect.current ?? new DOMRect(),
  });
  useEffect(() => {
    if (!open || !window.visualViewport) return;
    const viewport = window.visualViewport;
    let frame = 0;
    const reposition = (): void => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (props.anchor?.isConnected) {
          const rect = props.anchor.getBoundingClientRect();
          if (rect.width > 0 && rect.height > 0) anchorRect.current = rect;
        }
        position.current?.updatePosition();
      });
    };
    // iOS can animate the visual viewport without a window resize while the
    // software keyboard closes. Follow the send anchor without moving focus.
    viewport.addEventListener("resize", reposition);
    viewport.addEventListener("scroll", reposition);
    return () => {
      cancelAnimationFrame(frame);
      viewport.removeEventListener("resize", reposition);
      viewport.removeEventListener("scroll", reposition);
    };
  }, [open, props.anchor]);
  useEffect(() => {
    if (!open) return;
    let live = true;
    interacted.current = false;
    initializedFocus.current = false;
    setOptions(null);
    setError(null);
    void readDeliveryOptions(props.sessionId).then((value) => {
      if (live) setOptions(value);
    }).catch((reason: unknown) => {
      if (live) setError(String(reason));
    });
    return () => {
      live = false;
    };
  }, [open, props.sessionId]);
  const target = props.busy ? options?.target : null;
  const steer = target && options?.steer;
  const checkpoint = target && options?.controls.next_checkpoint;
  const unavailable = error
    ? "Runtime unavailable"
    : !options
    ? "Checking runtime…"
    : !props.busy
    ? "No active turn"
    : !options.steer
    ? "Not supported by this worker"
    : "Active turn unavailable";
  const mode = options?.policy.mode ?? "steer";
  const preferred = mode === "tool_boundary"
    ? "checkpoint"
    : mode === "steer"
    ? "steer"
    : "next";
  useEffect(() => {
    if (!open || !options || initializedFocus.current || interacted.current) {
      return;
    }
    initializedFocus.current = true;
    list.current?.querySelector<HTMLElement>(
      `[data-delivery-command="${preferred}"]`,
    )?.focus();
  }, [open, options, preferred]);
  // Keep a disabled row focusable after live invalidation. Activation must not
  // advance to a different action just because the original turn has ended.
  const choose = (id: string | undefined): void => {
    if (id === "steer" && steer) props.onSteer(target);
    else if (id === "checkpoint" && checkpoint) props.onSteer(target, true);
    else if (id === "next") props.onNextTurn();
    else if (id === "front" && props.queueCount > 0) props.onQueueFront();
    else if (id === "pause") props.onSendAndPause();
  };
  const commands = useRef({
    interact: (): void => {
      interacted.current = true;
    },
    activate: choose,
    close: props.onClose,
    stop: props.onStop,
  });
  commands.current = {
    interact: (): void => {
      interacted.current = true;
    },
    activate: choose,
    close: props.onClose,
    stop: props.onStop,
  };
  return (
    <Menu
      open={open}
      action={position}
      anchorEl={virtualAnchor.current}
      onClose={props.onClose}
      disableRestoreFocus
      autoFocus={false}
      disableAutoFocusItem
      anchorOrigin={{ vertical: "top", horizontal: "right" }}
      transformOrigin={{ vertical: "bottom", horizontal: "right" }}
      slotProps={{
        paper: {
          sx: {
            width: "max-content",
            minWidth: "max(200px, 14rem)",
            maxWidth: "calc(100vw - 24px)",
            mb: 1,
            borderRadius: "0.875rem",
            "& .MuiMenuItem-root": {
              minHeight: 40,
              px: 1.25,
              py: 0.75,
              gap: 1,
              fontSize: "0.875rem",
              borderRadius: "0.5rem",
              "&.Mui-disabled": {
                color: "text.disabled",
                bgcolor: "transparent",
                opacity: 1,
              },
            },
            // Desktop is a keyboard list: the cursor row is always visible,
            // including after a pointer opened the picker. It must outrank the
            // global touch rule that clears latched focus fills, which also
            // matches a touchscreen laptop (any-pointer: coarse).
            ...(desktop && {
              "& .MuiMenuItem-root:focus": {
                backgroundColor: (theme: Theme) =>
                  theme.palette.action.focus + " !important",
              },
            }),
            "& .MuiListItemText-root": { my: 0 },
            "& .MuiListItemIcon-root": { minWidth: "1.5rem", color: "inherit" },
            "& .MuiSvgIcon-root": { fontSize: "1.125rem" },
            "& .MuiListItemText-primary": { fontSize: "inherit" },
            "& .MuiListItemText-secondary": {
              color: "inherit",
              fontSize: "0.75rem",
              lineHeight: 1.4,
              whiteSpace: "normal",
            },
          },
        },
        list: {
          sx: { p: 0.5 },
          ref: (element: HTMLUListElement | null): void => {
            list.current = element;
            if (element) {
              element.dataset.deliveryPicker = "true";
              registerDeliveryPickerCommands(element, () => commands.current);
            }
            if (element && !initializedFocus.current && !interacted.current) {
              element.focus();
            }
          },
          tabIndex: -1,
          "aria-label": "Choose delivery",
          "aria-busy": !options && !error,
          disabledItemsFocusable: true,
          onKeyDownCapture: (
            event: ReactKeyboardEvent<HTMLUListElement>,
          ): void => {
            // Desktop handles this at capture in its central dispatcher;
            // mobile hardware keyboards use the same scoped registry here.
            if (event.nativeEvent.defaultPrevented) return;
            if (handleDeliveryPickerKey(event.nativeEvent)) {
              event.stopPropagation();
            }
          },
        },
      }}
    >
      <MenuItem
        data-delivery-command="steer"
        disabled={!steer}
        onClick={() => choose("steer")}
      >
        <ListItemIcon>
          <AltRoute />
        </ListItemIcon>
        <ListItemText
          primary={props.paused ? "Steer once" : "Steer"}
          secondary={!steer
            ? unavailable
            : props.paused
            ? "Queue stays paused"
            : undefined}
        />
      </MenuItem>
      <MenuItem
        data-delivery-command="next"
        onClick={() => choose("next")}
      >
        <ListItemIcon>
          <Schedule />
        </ListItemIcon>
        <ListItemText
          primary="Next turn"
          secondary={props.paused ? "Queue stays paused" : undefined}
        />
      </MenuItem>
      <MenuItem
        data-delivery-command="front"
        disabled={props.queueCount === 0}
        onClick={() => choose("front")}
      >
        <ListItemIcon>
          <FirstPage sx={{ transform: "rotate(90deg)" }} />
        </ListItemIcon>
        <ListItemText
          primary="Queue first"
          secondary={props.queueCount === 0
            ? "No queued messages"
            : props.paused
            ? "Queue stays paused"
            : undefined}
        />
      </MenuItem>
      {(options?.controls.next_checkpoint ||
        options?.policy.mode === "tool_boundary") && (
        <MenuItem
          data-delivery-command="checkpoint"
          disabled={!checkpoint}
          onClick={() => choose("checkpoint")}
        >
          <ListItemIcon>
            <SkipNext />
          </ListItemIcon>
          <ListItemText
            primary={props.paused
              ? "Next checkpoint · once"
              : "Next checkpoint"}
            secondary={!checkpoint
              ? "Not available on this worker"
              : props.paused
              ? "Queue stays paused"
              : undefined}
          />
        </MenuItem>
      )}
      <MenuItem data-delivery-command="pause" onClick={() => choose("pause")}>
        <ListItemIcon>
          <PauseCircleOutline />
        </ListItemIcon>
        <ListItemText
          primary="Send & pause"
          secondary="Enqueue · current turn continues"
        />
      </MenuItem>
      {desktop && (
        <Box
          component="li"
          role="presentation"
          data-delivery-picker-hints
          sx={{
            display: "flex",
            alignItems: "center",
            gap: 0.75,
            mt: 0.5,
            px: 1.25,
            pt: 0.75,
            pb: 0.25,
            typography: "caption",
            color: "text.secondary",
            fontSize: "0.6875rem",
            whiteSpace: "nowrap",
          }}
        >
          <DesktopShortcut shortcut="J/K" quiet />Move
          <DesktopShortcut shortcut="L" quiet />Send
          <DesktopShortcut shortcut="Esc" quiet />Close
        </Box>
      )}
    </Menu>
  );
}
