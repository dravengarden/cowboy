import {
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { createPortal } from "react-dom";
import { floatingIslandSurfaceSx } from "@cowboy/app-shell";
import { Box, Button, Tooltip } from "@mui/material";
import { alpha, type Theme } from "@mui/material/styles";
import Check from "@mui/icons-material/Check";
import Code from "@mui/icons-material/Code";
import ContentCopy from "@mui/icons-material/ContentCopy";
import { claimClipboardEpoch, copyRichTextDetailed, copyText, isCurrentClipboardEpoch } from "./clipboard";
import { writeVimRegister } from "./desktop/vim/vimRegister";
import type { RenderItem } from "./derive";
import { magneticHaptic } from "./haptic";
import { messageBubbleBorderRadius } from "./messageBubble";
import { extractCopyableMessage, formatMessageForCopy } from "./messageCopyFormat";
import {
  closeMobileMessageActions,
  getMobileMessageActions,
  hasActiveTextSelection,
  type MessageCopyMode,
  openMobileMessageActions,
  registerMessageCopyHandler,
  subscribeMobileMessageActions,
} from "./messageCopyRegistry";
import { MessageForkButton, MessageWithFork } from "./MessageForkAction";
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
  offsetLeft = 0,
  offsetTop = 0,
): { left: number; top: number } {
  const hMargin = Math.min(12, Math.max(4, Math.floor((viewportWidth - pillWidth) / 2)));
  const clampedWidth = Math.min(pillWidth, Math.max(0, viewportWidth - hMargin * 2));
  const minLeft = offsetLeft + hMargin;
  const maxLeft = Math.max(minLeft, offsetLeft + viewportWidth - clampedWidth - hMargin);
  const left = Math.min(Math.max(minLeft, Math.round(tapX - clampedWidth / 2)), maxLeft);

  const slackY = Math.max(0, viewportHeight - pillHeight);
  const topClear = slackY >= 152 ? 56 : Math.min(56, Math.floor(slackY * 0.35));
  const bottomClear = slackY >= 152 ? 76 : Math.min(76, Math.floor(slackY * 0.45));
  const minTop = offsetTop + topClear;
  const maxTop = Math.max(minTop, offsetTop + viewportHeight - pillHeight - bottomClear);
  const preferAbove = tapY - pillHeight - 14;
  const topCandidate = preferAbove >= minTop ? preferAbove : tapY + 16;
  const top = Math.min(Math.max(minTop, Math.round(topCandidate)), maxTop);
  return { left, top };
}

function pillButtonSx(activeSuccess: boolean, mobile: boolean): (theme: Theme) => Record<string, unknown> {
  return (theme) => ({
    minWidth: 0,
    minHeight: mobile ? 40 : "1.375rem",
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
      aria-hidden={focusable ? undefined : true}
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
        // Primary bottom pill uses opacity (never visibility: hidden) so
        // Desktop Tab traversal can focus its action stops and reveal the bar
        // via :focus-within. Decorative top mirror stays visibility: hidden.
        ...(focusable
          ? {
              opacity: keepVisible ? 1 : 0,
              pointerEvents: keepVisible ? "auto" : "none",
            }
          : {
              visibility: keepVisible ? "visible" : "hidden",
              opacity: keepVisible ? 1 : 0,
              pointerEvents: keepVisible ? "auto" : "none",
            }),
        transition: "opacity 120ms ease",
        ".cowboy-msg-wrap:hover &, .cowboy-msg-wrap:focus-within &": {
          ...(focusable ? {} : { visibility: "visible" }),
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
          aria-keyshortcuts={focusable ? "y" : undefined}
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
          aria-keyshortcuts={focusable ? "Shift+Y" : undefined}
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
  const pillRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<{ w: number; h: number }>({ w: 228, h: 44 });

  useLayoutEffect(() => {
    const el = pillRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const w = Math.ceil(rect.width);
    const h = Math.ceil(rect.height);
    if (w > 0 && h > 0 && (w !== size.w || h !== size.h)) {
      setSize({ w, h });
    }
  }, [copiedKind, size.h, size.w]);

  const copyTap = useReliableTouchTap<HTMLButtonElement>(() => onCopy("rich"));
  const mdTap = useReliableTouchTap<HTMLButtonElement>(() => onCopy("markdown"));

  if (typeof document === "undefined") return null;
  const vv = globalThis.visualViewport;
  const vw = vv?.width || globalThis.innerWidth || 390;
  const vh = vv?.height || globalThis.innerHeight || 844;
  const offsetLeft = vv?.offsetLeft ?? 0;
  const offsetTop = vv?.offsetTop ?? 0;
  const pos = clampMobilePillPosition(x, y, vw, vh, size.w, size.h, offsetLeft, offsetTop);
  const copyDone = copiedKind === "rich";
  const mdDone = copiedKind === "markdown";

  return createPortal(
    <Box
      ref={pillRef}
      role="toolbar"
      aria-label="Message copy actions"
      data-message-action-bar="mobile"
      sx={(theme) => ({
        position: "fixed",
        left: pos.left,
        top: pos.top,
        zIndex: 1400,
        display: "inline-flex",
        flexWrap: "wrap",
        justifyContent: "center",
        alignItems: "center",
        maxWidth: `${Math.max(88, Math.floor(vw - 8))}px`,
        borderRadius: size.h > 52 ? "16px" : "999px",
        overflow: "hidden",
        ...floatingIslandSurfaceSx(theme, { compact: true }),
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
  onContextMenu?: () => void;
} {
  const tapRef = useRef<{ pointerId: number; x: number; y: number; at: number } | null>(null);
  const onPointerDown = useCallback((event: ReactPointerEvent<HTMLDivElement>): void => {
    if (
      !enabled ||
      !event.isPrimary ||
      (event.pointerType !== "touch" && event.button !== 0) ||
      isInteractiveMessageTarget(event.target, event.currentTarget)
    ) {
      tapRef.current = null;
      return;
    }
    if (hasActiveTextSelection()) {
      tapRef.current = null;
      return;
    }
    tapRef.current = { pointerId: event.pointerId, x: event.clientX, y: event.clientY, at: Date.now() };
  }, [enabled]);

  const onPointerMove = useCallback((event: ReactPointerEvent<HTMLDivElement>): void => {
    const start = tapRef.current;
    if (!start || start.pointerId !== event.pointerId) return;
    if (Math.hypot(event.clientX - start.x, event.clientY - start.y) > RELIABLE_TOUCH_TAP_MOVE_SLOP_PX) {
      tapRef.current = null;
      closeMobileMessageActions();
    }
  }, []);

  const onPointerUp = useCallback((event: ReactPointerEvent<HTMLDivElement>): void => {
    const start = tapRef.current;
    tapRef.current = null;
    if (!start || start.pointerId !== event.pointerId) return;
    if (Date.now() - start.at > MOBILE_TAP_MAX_MS) return;
    if (Math.hypot(event.clientX - start.x, event.clientY - start.y) > RELIABLE_TOUCH_TAP_MOVE_SLOP_PX) return;
    if (hasActiveTextSelection()) return;
    if (getMobileMessageActions()?.copyId === copyId) {
      closeMobileMessageActions(copyId);
    } else {
      openMobileMessageActions(copyId, event.clientX, event.clientY);
    }
  }, [copyId]);

  const clearTap = useCallback((): void => {
    tapRef.current = null;
  }, []);

  return enabled
    ? { onPointerDown, onPointerMove, onPointerUp, onPointerCancel: clearTap, onContextMenu: clearTap }
    : {};
}

function useMessageCopyRunner(
  copyId: string,
  desktop: boolean,
  markdown: string,
  enabled: boolean,
): { copiedKind: MessageCopyMode | null; runCopy: (mode: MessageCopyMode) => Promise<boolean> } {
  const [copiedKind, setCopiedKind] = useState<MessageCopyMode | null>(null);
  const copiedTimerRef = useRef<ReturnType<typeof globalThis.setTimeout> | undefined>(undefined);
  const mobileDismissTimerRef = useRef<ReturnType<typeof globalThis.setTimeout> | undefined>(undefined);
  const preferPlainRetryRef = useRef(false);

  useEffect(() => {
    preferPlainRetryRef.current = false;
  }, [markdown]);

  const runCopy = useCallback(async (mode: MessageCopyMode): Promise<boolean> => {
    if (!markdown.trim()) return false;
    const epoch = claimClipboardEpoch();
    let ok = false;
    if (mode === "markdown") {
      if (desktop) writeVimRegister(markdown, false);
      ok = await copyText(markdown, epoch);
    } else {
      const { html, plain } = formatMessageForCopy(markdown);
      const plainText = plain.trim() ? plain : markdown;
      if (desktop) writeVimRegister(plainText, false);
      if (preferPlainRetryRef.current) {
        ok = await copyText(plainText, epoch);
        if (ok && isCurrentClipboardEpoch(epoch)) preferPlainRetryRef.current = false;
      } else {
        const res = await copyRichTextDetailed(plainText, html, epoch);
        if (res.superseded || !isCurrentClipboardEpoch(epoch)) return false;
        ok = res.ok;
        if (!ok && res.retryPlainText) {
          preferPlainRetryRef.current = true;
          notify("Could not copy formatted text — tap Copy again for plain text", "info");
          return false;
        }
      }
    }
    if (!isCurrentClipboardEpoch(epoch)) return false;
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
    if (!enabled) return undefined;
    return registerMessageCopyHandler(copyId, runCopy);
  }, [copyId, enabled, runCopy]);

  useEffect(() => () => {
    if (copiedTimerRef.current !== undefined) globalThis.clearTimeout(copiedTimerRef.current);
    if (mobileDismissTimerRef.current !== undefined) globalThis.clearTimeout(mobileDismissTimerRef.current);
    closeMobileMessageActions(copyId);
  }, [copyId]);

  return { copiedKind, runCopy };
}

/** Unified hidden Copy / Copy as Markdown (and Desktop turn Fork) affordance
 *  for completed transcript messages across Desktop and Mobile. */
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
  const [tall, setTall] = useState(false);

  const mobileActive = useSyncExternalStore(
    subscribeMobileMessageActions,
    getMobileMessageActions,
    () => null,
  );
  const mobileOpen = !desktop && copyable !== null && mobileActive?.copyId === copyId;
  const { copiedKind, runCopy } = useMessageCopyRunner(
    copyId,
    desktop,
    copyable?.markdown ?? "",
    copyable !== null,
  );

  const hasCopyable = copyable !== null;
  useEffect(() => {
    if (!desktop || !hasCopyable || typeof ResizeObserver === "undefined") {
      setTall(false);
      return undefined;
    }
    const el = wrapperRef.current;
    if (!el) return undefined;
    const update = (height: number): void => {
      setTall(height > TALL_MESSAGE_THRESHOLD_PX);
    };
    update(el.getBoundingClientRect().height);
    const ro = new ResizeObserver(([entry]) => {
      if (entry) update(entry.contentRect.height);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [desktop, hasCopyable]);

  const mobileTap = useMobileMessageTap(copyId, !desktop && copyable !== null);
  const allowFork = desktop && !readOnly && item.role === "assistant" && !streaming && Boolean(forkPoint);

  if (!copyable) {
    if (!allowFork) return children;
    return (
      <MessageWithFork
        item={item}
        sessionId={sessionId}
        streaming={streaming}
        desktop={desktop}
        forkPoint={forkPoint}
      >
        {children}
      </MessageWithFork>
    );
  }

  const isUser = copyable.role === "user";
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
