import {
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { createPortal } from "react-dom";
import { Box, Button, Tooltip } from "@mui/material";
import { alpha, type Theme } from "@mui/material/styles";
import Check from "@mui/icons-material/Check";
import Code from "@mui/icons-material/Code";
import ContentCopy from "@mui/icons-material/ContentCopy";
import { copyRichText, copyText } from "./clipboard";
import { writeVimRegister } from "./desktop/vim/vimRegister";
import type { RenderItem } from "./derive";
import { magneticHaptic } from "./haptic";
import { messageBubbleBorderRadius } from "./messageBubble";
import { extractCopyableMessage, formatMessageForCopy } from "./messageCopyFormat";
import {
  closeMobileMessageActions,
  getMobileMessageActions,
  type MessageCopyMode,
  openMobileMessageActions,
  registerMessageCopyHandler,
  subscribeMobileMessageActions,
} from "./messageCopyRegistry";
import { MessageForkButton } from "./MessageForkAction";
import { notify, useStoreSelector } from "./store";
import { glyphSize } from "./surface/desktopSize";
import { RELIABLE_TOUCH_TAP_MOVE_SLOP_PX } from "./touchGestures";
import type { TurnForkPoint } from "./turnForkPoints";
import { useReliableTouchTap } from "./useReliableTouchTap";

const TALL_MESSAGE_THRESHOLD_PX = 480;
const MOBILE_TAP_MAX_MS = 450;
const COPIED_FEEDBACK_MS = 1500;
const MOBILE_DISMISS_AFTER_COPY_MS = 650;

const INTERACTIVE_PROSE_SELECTOR = [
  "a",
  "button",
  "input",
  "select",
  "textarea",
  "summary",
  "details",
  "img",
  "[role='button']",
  "[data-message-action-bar]",
  ".cowboy-copy-btn",
].join(", ");

export function isInteractiveMessageTarget(target: EventTarget | null, wrapper: HTMLElement): boolean {
  if (!(target instanceof Element)) return false;
  const hit = target.closest(INTERACTIVE_PROSE_SELECTOR);
  return hit !== null && hit !== wrapper;
}

export function clampMobilePillPosition(
  tapX: number,
  tapY: number,
  viewportWidth: number,
  viewportHeight: number,
  pillWidth = 228,
  pillHeight = 44,
): { left: number; top: number } {
  const margin = 12;
  const maxLeft = Math.max(margin, viewportWidth - pillWidth - margin);
  const left = Math.min(Math.max(margin, Math.round(tapX - pillWidth / 2)), maxLeft);
  const preferAbove = tapY - pillHeight - 14;
  const topCandidate = preferAbove >= 56 ? preferAbove : tapY + 16;
  const maxTop = Math.max(56, viewportHeight - pillHeight - 76);
  const top = Math.min(Math.max(56, Math.round(topCandidate)), maxTop);
  return { left, top };
}

function pillButtonSx(activeSuccess: boolean, mobile: boolean): (theme: Theme) => Record<string, unknown> {
  return (theme) => ({
    minWidth: 0,
    minHeight: mobile ? 40 : 22,
    px: mobile ? 1.5 : 0.75,
    py: 0,
    borderRadius: 0,
    fontSize: mobile ? "0.8125rem" : "0.7rem",
    fontWeight: mobile ? 500 : 450,
    lineHeight: 1.2,
    color: activeSuccess ? "success.main" : "text.secondary",
    bgcolor: "transparent",
    textTransform: "none",
    whiteSpace: "nowrap",
    "& .MuiButton-startIcon": { mr: mobile ? 0.6 : 0.4 },
    "&:hover": {
      color: activeSuccess ? "success.main" : "primary.main",
      bgcolor: alpha(
        activeSuccess ? theme.palette.success.main : theme.palette.primary.main,
        theme.palette.mode === "dark" ? 0.14 : 0.08,
      ),
    },
  });
}

function HairlineDivider(): React.JSX.Element {
  return (
    <Box
      aria-hidden
      sx={(theme) => ({
        width: "1px",
        alignSelf: "stretch",
        my: 0.45,
        bgcolor: alpha(theme.palette.divider, 0.85),
        flexShrink: 0,
      })}
    />
  );
}

function DesktopHoverPill({
  position,
  alignRightEdge,
  copiedKind,
  onCopy,
  originSessionId,
  forkPoint,
  focusable,
}: {
  position: "bottom" | "top";
  alignRightEdge: number;
  copiedKind: MessageCopyMode | null;
  onCopy: (mode: MessageCopyMode) => void;
  originSessionId?: string | undefined;
  forkPoint?: TurnForkPoint | undefined;
  focusable: boolean;
}): React.JSX.Element {
  const forkEligible = useStoreSelector((state) => {
    if (!originSessionId || !forkPoint) return false;
    const session = state.sessions.find((entry) => entry.id === originSessionId);
    return Boolean(session && !session.system && session.workspace_id);
  });
  const keepVisible = copiedKind !== null;
  const copyDone = copiedKind === "rich";
  const mdDone = copiedKind === "markdown";

  return (
    <Box
      data-message-action-bar={position}
      data-message-fork-bar={forkEligible ? "" : undefined}
      sx={(theme) => ({
        position: "absolute",
        right: alignRightEdge,
        ...(position === "bottom" ? { bottom: -11 } : { top: -11 }),
        zIndex: 2,
        display: "inline-flex",
        alignItems: "center",
        borderRadius: "999px",
        overflow: "hidden",
        bgcolor: theme.palette.background.paper,
        border: `1px solid ${alpha(theme.palette.divider, 0.9)}`,
        boxShadow: theme.shadows[1],
        visibility: keepVisible ? "visible" : "hidden",
        opacity: keepVisible ? 1 : 0,
        pointerEvents: keepVisible ? "auto" : "none",
        transition: "opacity 120ms ease",
        ".cowboy-msg-wrap:hover &, .cowboy-msg-wrap:focus-within &": {
          visibility: "visible",
          opacity: 1,
          pointerEvents: "auto",
        },
      })}
    >
      <Tooltip title={copyDone ? "Copied" : "Copy (y)"}>
        <Button
          size="small"
          variant="text"
          color="inherit"
          tabIndex={focusable ? 0 : -1}
          data-message-copy="rich"
          data-desktop-transcript-stop={focusable ? "action" : undefined}
          aria-label={copyDone ? "Copied" : "Copy message"}
          startIcon={
            copyDone
              ? <Check sx={{ fontSize: glyphSize(13) }} />
              : <ContentCopy sx={{ fontSize: glyphSize(12) }} />
          }
          onClick={(): void => onCopy("rich")}
          sx={pillButtonSx(copyDone, false)}
        >
          {copyDone ? "Copied" : "Copy"}
        </Button>
      </Tooltip>
      <HairlineDivider />
      <Tooltip title={mdDone ? "Copied as Markdown" : "Copy as Markdown (Y)"}>
        <Button
          size="small"
          variant="text"
          color="inherit"
          tabIndex={focusable ? 0 : -1}
          data-message-copy="markdown"
          data-desktop-transcript-stop={focusable ? "action" : undefined}
          aria-label={mdDone ? "Copied as Markdown" : "Copy message as Markdown"}
          startIcon={
            mdDone
              ? <Check sx={{ fontSize: glyphSize(13) }} />
              : <Code sx={{ fontSize: glyphSize(13) }} />
          }
          onClick={(): void => onCopy("markdown")}
          sx={pillButtonSx(mdDone, false)}
        >
          {mdDone ? "Copied MD" : "Markdown"}
        </Button>
      </Tooltip>
      {forkEligible && originSessionId && forkPoint && (
        <>
          <HairlineDivider />
          <MessageForkButton originSessionId={originSessionId} forkPoint={forkPoint} inline />
        </>
      )}
    </Box>
  );
}

function MobileFloatingCopyPill({
  x,
  y,
  copiedKind,
  onCopy,
}: {
  x: number;
  y: number;
  copiedKind: MessageCopyMode | null;
  onCopy: (mode: MessageCopyMode) => void;
}): React.JSX.Element | null {
  if (typeof document === "undefined") return null;
  const vw = globalThis.innerWidth || 390;
  const vh = globalThis.innerHeight || 844;
  const pos = clampMobilePillPosition(x, y, vw, vh);
  const copyDone = copiedKind === "rich";
  const mdDone = copiedKind === "markdown";

  const copyTap = useReliableTouchTap<HTMLButtonElement>(() => onCopy("rich"));
  const mdTap = useReliableTouchTap<HTMLButtonElement>(() => onCopy("markdown"));

  return createPortal(
    <Box
      role="toolbar"
      aria-label="Message copy actions"
      data-message-action-bar="mobile"
      sx={(theme) => ({
        position: "fixed",
        left: pos.left,
        top: pos.top,
        zIndex: 1400,
        display: "inline-flex",
        alignItems: "center",
        borderRadius: "999px",
        overflow: "hidden",
        bgcolor: alpha(theme.palette.background.paper, 0.96),
        backdropFilter: "blur(14px)",
        WebkitBackdropFilter: "blur(14px)",
        border: `1px solid ${alpha(theme.palette.divider, 0.95)}`,
        boxShadow: theme.shadows[6],
      })}
    >
      <Button
        size="small"
        variant="text"
        color="inherit"
        data-message-copy="rich"
        aria-label={copyDone ? "Copied" : "Copy message"}
        startIcon={
          copyDone
            ? <Check sx={{ fontSize: glyphSize(16) }} />
            : <ContentCopy sx={{ fontSize: glyphSize(15) }} />
        }
        {...copyTap}
        sx={pillButtonSx(copyDone, true)}
      >
        {copyDone ? "Copied" : "Copy"}
      </Button>
      <HairlineDivider />
      <Button
        size="small"
        variant="text"
        color="inherit"
        data-message-copy="markdown"
        aria-label={mdDone ? "Copied as Markdown" : "Copy message as Markdown"}
        startIcon={
          mdDone
            ? <Check sx={{ fontSize: glyphSize(16) }} />
            : <Code sx={{ fontSize: glyphSize(16) }} />
        }
        {...mdTap}
        sx={pillButtonSx(mdDone, true)}
      >
        {mdDone ? "Copied MD" : "Markdown"}
      </Button>
    </Box>,
    document.body,
  );
}

function useMobileMessageTap(
  copyId: string,
  enabled: boolean,
): {
  onPointerDown?: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onPointerMove?: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onPointerUp?: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onPointerCancel?: (event: ReactPointerEvent<HTMLDivElement>) => void;
} {
  const tapRef = useRef<{ pointerId: number; x: number; y: number; at: number } | null>(null);
  const onPointerDown = useCallback((event: ReactPointerEvent<HTMLDivElement>): void => {
    if (!enabled || !event.isPrimary || isInteractiveMessageTarget(event.target, event.currentTarget)) {
      tapRef.current = null;
      return;
    }
    if ((globalThis.getSelection?.()?.toString() ?? "").trim().length > 0) {
      tapRef.current = null;
      return;
    }
    tapRef.current = { pointerId: event.pointerId, x: event.clientX, y: event.clientY, at: event.timeStamp };
  }, [enabled]);
  const onPointerMove = useCallback((event: ReactPointerEvent<HTMLDivElement>): void => {
    const start = tapRef.current;
    if (!start || start.pointerId !== event.pointerId) return;
    if (
      Math.abs(event.clientX - start.x) > RELIABLE_TOUCH_TAP_MOVE_SLOP_PX ||
      Math.abs(event.clientY - start.y) > RELIABLE_TOUCH_TAP_MOVE_SLOP_PX
    ) {
      tapRef.current = null;
    }
  }, []);
  const onPointerUp = useCallback((event: ReactPointerEvent<HTMLDivElement>): void => {
    const start = tapRef.current;
    tapRef.current = null;
    if (!start || start.pointerId !== event.pointerId || event.timeStamp - start.at > MOBILE_TAP_MAX_MS) return;
    if ((globalThis.getSelection?.()?.toString() ?? "").trim().length > 0) return;
    if (getMobileMessageActions()?.copyId === copyId) closeMobileMessageActions(copyId);
    else openMobileMessageActions(copyId, event.clientX, event.clientY);
  }, [copyId]);
  const onPointerCancel = useCallback((event: ReactPointerEvent<HTMLDivElement>): void => {
    if (tapRef.current?.pointerId === event.pointerId) tapRef.current = null;
  }, []);
  return enabled ? { onPointerDown, onPointerMove, onPointerUp, onPointerCancel } : {};
}

/** Unified discreet message action wrapper for both Desktop and Mobile:
 *  - Desktop: zero-height bottom-right hover/focus capsule combining Copy,
 *    Copy as Markdown, and (on mutable turn-final assistant replies) Fork from
 *    here; tall replies (> 480px) mirror Copy/Markdown straddling the outer
 *    top edge; Vim `y`/`Y` in Conversation invoke the registered handler.
 *  - Mobile: zero chrome at rest; native iOS long-press text selection stays
 *    untouched; a stationary tap on non-interactive prose pops a viewport-safe
 *    floating [ Copy | Markdown ] pill near the thumb. */
export function MessageWithActions({
  item,
  sessionId,
  streaming,
  desktop,
  readOnly = false,
  forkPoint,
  children,
}: {
  item: Extract<RenderItem, { kind: "message" }>;
  sessionId: string;
  streaming: boolean;
  desktop: boolean;
  readOnly?: boolean | undefined;
  forkPoint?: TurnForkPoint | undefined;
  children: React.JSX.Element;
}): React.JSX.Element {
  const copyable = streaming ? null : extractCopyableMessage(item);
  const copyId = `${sessionId}:${readOnly ? "anc:" : ""}${item.key}`;
  const wrapperRef = useRef<HTMLDivElement>(null);
  const [copiedKind, setCopiedKind] = useState<MessageCopyMode | null>(null);
  const [tall, setTall] = useState(false);
  const copiedTimerRef = useRef<ReturnType<typeof globalThis.setTimeout> | undefined>(undefined);
  const mobileDismissTimerRef = useRef<ReturnType<typeof globalThis.setTimeout> | undefined>(undefined);

  const mobileActive = useSyncExternalStore(
    subscribeMobileMessageActions,
    getMobileMessageActions,
    () => null,
  );
  const mobileOpen = !desktop && copyable !== null && mobileActive?.copyId === copyId;

  const markdown = copyable?.markdown ?? "";

  const runCopy = useCallback(async (mode: MessageCopyMode): Promise<boolean> => {
    if (!markdown.trim()) return false;
    let ok = false;
    if (mode === "markdown") {
      ok = await copyText(markdown);
      if (ok && desktop) writeVimRegister(markdown, false);
    } else {
      const { html, plain } = formatMessageForCopy(markdown);
      ok = await copyRichText(plain, html);
      if (ok && desktop) writeVimRegister(plain, false);
    }
    if (!ok) {
      notify("Could not copy to clipboard", "error");
      return false;
    }
    if (!desktop) magneticHaptic();
    setCopiedKind(mode);
    if (copiedTimerRef.current !== undefined) globalThis.clearTimeout(copiedTimerRef.current);
    copiedTimerRef.current = globalThis.setTimeout(() => {
      setCopiedKind(null);
      copiedTimerRef.current = undefined;
    }, COPIED_FEEDBACK_MS);
    if (!desktop) {
      if (mobileDismissTimerRef.current !== undefined) globalThis.clearTimeout(mobileDismissTimerRef.current);
      mobileDismissTimerRef.current = globalThis.setTimeout(() => {
        closeMobileMessageActions(copyId);
        mobileDismissTimerRef.current = undefined;
      }, MOBILE_DISMISS_AFTER_COPY_MS);
    }
    return true;
  }, [copyId, desktop, markdown]);

  useEffect(() => {
    if (!copyable) return undefined;
    return registerMessageCopyHandler(copyId, runCopy);
  }, [copyId, copyable, runCopy]);

  useEffect(() => () => {
    if (copiedTimerRef.current !== undefined) globalThis.clearTimeout(copiedTimerRef.current);
    if (mobileDismissTimerRef.current !== undefined) globalThis.clearTimeout(mobileDismissTimerRef.current);
    closeMobileMessageActions(copyId);
  }, [copyId]);

  useEffect(() => {
    if (!desktop || !copyable || typeof ResizeObserver === "undefined") {
      setTall(false);
      return undefined;
    }
    const el = wrapperRef.current;
    if (!el) return undefined;
    const ro = new ResizeObserver(([entry]) => {
      if (entry) setTall(entry.contentRect.height > TALL_MESSAGE_THRESHOLD_PX);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [desktop, copyable]);

  const mobileTap = useMobileMessageTap(copyId, !desktop && copyable !== null);

  if (!copyable) return children;

  const isUser = copyable.role === "user";
  const allowFork = desktop && !readOnly && copyable.role === "assistant" && Boolean(forkPoint);
  const alignRightEdge = isUser ? 0 : 10;

  return (
    <Box
      ref={wrapperRef}
      className="cowboy-msg-wrap"
      data-message-copy-id={copyId}
      data-message-copy-role={copyable.role}
      {...mobileTap}
      sx={(theme) => ({
        position: "relative",
        alignSelf: isUser ? "flex-end" : "stretch",
        width: isUser ? "fit-content" : "100%",
        maxWidth: "100%",
        minWidth: 0,
        borderRadius: messageBubbleBorderRadius(copyable.role),
        ...(mobileOpen && {
          boxShadow: `0 0 0 1.5px ${alpha(theme.palette.primary.main, 0.38)}`,
        }),
      })}
    >
      {children}
      {desktop && (
        <DesktopHoverPill
          position="bottom"
          alignRightEdge={alignRightEdge}
          copiedKind={copiedKind}
          onCopy={(mode): void => { void runCopy(mode); }}
          originSessionId={allowFork ? sessionId : undefined}
          forkPoint={allowFork ? forkPoint : undefined}
          focusable
        />
      )}
      {desktop && tall && (
        <DesktopHoverPill
          position="top"
          alignRightEdge={alignRightEdge}
          copiedKind={copiedKind}
          onCopy={(mode): void => { void runCopy(mode); }}
          focusable={false}
        />
      )}
      {mobileOpen && mobileActive && (
        <MobileFloatingCopyPill
          x={mobileActive.x}
          y={mobileActive.y}
          copiedKind={copiedKind}
          onCopy={(mode): void => { void runCopy(mode); }}
        />
      )}
    </Box>
  );
}
