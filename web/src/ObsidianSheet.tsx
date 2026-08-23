// Cowboy compact modal — a flush bottom sheet, cowboy-tinted.
//
// Shared DetentSheet stays the cover/workbench surface (Settings, New Session).
// Compact decisions and inspectors (Confirm, Symbols, Info, Move draft) use
// this card: full-bleed left/right, docked to the bottom, safe-area padded
// INSIDE so the home indicator sits in the card instead of floating it.
// Content-hugging. Drag is 1:1; release is the iOS cubic.

import {
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { Box, Paper, Typography, useTheme } from "@mui/material";
import { alpha } from "@mui/material/styles";
import { markDetentSheetOpen } from "./components/app-shell/detent-sheet-open.ts";
import { haptic as fireHaptic, MobileSheetDismiss } from "./components/app-shell";
import {
  OBSIDIAN_SHEET_INSET_PX,
  OBSIDIAN_SHEET_MAX_FRACTION,
  OBSIDIAN_SHEET_RADIUS_PX,
  OBSIDIAN_SHEET_SCRIM_MAX,
  OBSIDIAN_SHEET_SETTLE_EASING,
  obsidianSheetScale,
  obsidianSheetScrimOpacity,
  obsidianSheetScrimPointerEvents,
  obsidianSheetSettleMs,
  obsidianSheetTransform,
} from "./obsidianSheetMotion";
import { useBackdropDismiss } from "./useBackdropDismiss";

const Z = 1250;
const PROJECTION_MS = 110;
const FLICK_DISMISS = 0.55;
const HANDLE_HEIGHT = 18;
const SAFE_INSIDE =
  "max(8px, calc(env(safe-area-inset-bottom, 0px) - var(--kb-inset, 0px)))";

function dimHex(base: string, dim: number): string {
  const hex = base.trim().replace(/^#/u, "");
  const full = hex.length === 3 ? [...hex].map((c) => c + c).join("") : hex;
  if (full.length !== 6 || /[^0-9a-f]/iu.test(full)) return base;
  const factor = 1 - Math.min(1, Math.max(0, dim));
  const channel = (index: number): string =>
    Math.round(Number.parseInt(full.slice(index, index + 2), 16) * factor)
      .toString(16)
      .padStart(2, "0");
  return `#${channel(0)}${channel(2)}${channel(4)}`;
}

function setStatusBarColor(color: string): void {
  const { head } = globalThis.document;
  if (!head) return;
  for (const meta of head.querySelectorAll('meta[name="theme-color"]')) {
    meta.remove();
  }
  const meta = globalThis.document.createElement("meta");
  meta.setAttribute("name", "theme-color");
  meta.setAttribute("content", color);
  head.append(meta);
}

function prefersReducedMotion(): boolean {
  return globalThis.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches ===
    true;
}

export interface ObsidianSheetProps {
  readonly open: boolean;
  readonly onClose: () => void;
  readonly title?: ReactNode | undefined;
  readonly children: ReactNode;
  readonly actions?: ReactNode | undefined;
  readonly ariaLabel?: string | undefined;
  readonly floatingDismiss?: boolean | undefined;
}

export function ObsidianSheet({
  open,
  onClose,
  title,
  children,
  actions,
  ariaLabel,
  floatingDismiss = false,
}: ObsidianSheetProps): ReactNode {
  const theme = useTheme();
  const surfaceColor = theme.palette.background.default;
  const sheetRef = useRef<HTMLDivElement | null>(null);
  const scrimRef = useRef<HTMLDivElement | null>(null);
  const yRef = useRef(0);
  const closedPxRef = useRef(0);
  const animatedRef = useRef(false);
  const dragRef = useRef<
    { startPointerY: number; startY: number; samples: { t: number; y: number }[] } | null
  >(null);
  const rafRef = useRef(0);
  const pendingYRef = useRef<number | null>(null);
  const movingTimerRef = useRef<ReturnType<typeof globalThis.setTimeout> | null>(
    null,
  );
  const onCloseRef = useRef(onClose);
  const dismissingRef = useRef(false);
  onCloseRef.current = onClose;

  useEffect(() => {
    setStatusBarColor(
      open
        ? dimHex(surfaceColor, OBSIDIAN_SHEET_SCRIM_MAX)
        : surfaceColor,
    );
    return () => {
      if (open) setStatusBarColor(surfaceColor);
    };
  }, [open, surfaceColor]);

  const paint = useCallback((y: number, animate: boolean): void => {
    yRef.current = y;
    const closedPx = closedPxRef.current;
    const sheet = sheetRef.current;
    const scrim = scrimRef.current;
    const settle = obsidianSheetSettleMs(prefersReducedMotion());
    const scale = prefersReducedMotion() ? 1 : obsidianSheetScale(y, closedPx);
    if (sheet) {
      sheet.dataset["detentMoving"] = "";
      if (movingTimerRef.current !== null) {
        globalThis.clearTimeout(movingTimerRef.current);
        movingTimerRef.current = null;
      }
      const transition = animate
        ? `transform ${String(settle)}ms ${OBSIDIAN_SHEET_SETTLE_EASING}`
        : "none";
      sheet.style.transition = transition;
      sheet.style.transform = obsidianSheetTransform(y, scale);
      if (animate) {
        movingTimerRef.current = globalThis.setTimeout(() => {
          delete sheet.dataset["detentMoving"];
          movingTimerRef.current = null;
        }, settle + 34);
      }
    }
    if (scrim) {
      const opacity = obsidianSheetScrimOpacity(y, closedPx);
      scrim.style.transition = animate
        ? `opacity ${String(settle)}ms ${OBSIDIAN_SHEET_SETTLE_EASING}`
        : "none";
      scrim.style.opacity = String(opacity);
      // A closing scrim must keep owning the pointer stream until unmount.
      // Otherwise iOS WebKit's delayed synthetic click can hit the code below
      // and open another inspector while this sheet is disappearing.
      scrim.style.pointerEvents = obsidianSheetScrimPointerEvents(
        opacity,
        dismissingRef.current,
      );
    }
  }, []);

  useEffect(() => () => {
    if (movingTimerRef.current !== null) {
      globalThis.clearTimeout(movingTimerRef.current);
    }
  }, []);

  useEffect(() => {
    if (!open) {
      animatedRef.current = false;
      dismissingRef.current = false;
      return;
    }
    const measure = (): number =>
      sheetRef.current?.offsetHeight ??
        OBSIDIAN_SHEET_MAX_FRACTION * globalThis.innerHeight;
    closedPxRef.current = measure();
    if (animatedRef.current) {
      paint(0, true);
      return;
    }
    fireHaptic("medium");
    paint(closedPxRef.current, false);
    const id = globalThis.requestAnimationFrame(() => {
      animatedRef.current = true;
      paint(0, true);
    });
    const sheet = sheetRef.current;
    const ro = typeof ResizeObserver === "undefined" || sheet === null
      ? null
      : new ResizeObserver(() => {
        closedPxRef.current = measure();
      });
    if (ro !== null && sheet !== null) {
      ro.observe(sheet);
    }
    return () => {
      globalThis.cancelAnimationFrame(id);
      ro?.disconnect();
    };
  }, [open, paint]);

  const dismiss = useCallback((): void => {
    if (dismissingRef.current) return;
    dismissingRef.current = true;
    fireHaptic("light");
    paint(closedPxRef.current, true);
    globalThis.setTimeout(
      () => onCloseRef.current(),
      obsidianSheetSettleMs(prefersReducedMotion()),
    );
  }, [paint]);
  const dismissBackdrop = useBackdropDismiss<HTMLDivElement>(dismiss);

  const onPointerDown = useCallback((e: ReactPointerEvent<HTMLDivElement>): void => {
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    dragRef.current = {
      startPointerY: e.clientY,
      startY: yRef.current,
      samples: [{ t: e.timeStamp, y: e.clientY }],
    };
  }, []);

  const onPointerMove = useCallback((e: ReactPointerEvent<HTMLDivElement>): void => {
    const d = dragRef.current;
    if (!d) return;
    let y = d.startY + (e.clientY - d.startPointerY);
    if (y < 0) y *= 0.12;
    y = Math.min(y, closedPxRef.current);
    d.samples.push({ t: e.timeStamp, y: e.clientY });
    if (d.samples.length > 5) d.samples.shift();
    pendingYRef.current = y;
    if (rafRef.current === 0) {
      rafRef.current = globalThis.requestAnimationFrame(() => {
        rafRef.current = 0;
        const pendingY = pendingYRef.current;
        pendingYRef.current = null;
        if (pendingY !== null) paint(pendingY, false);
      });
    }
  }, [paint]);

  const finishDrag = useCallback((projectVelocity: boolean): void => {
    const d = dragRef.current;
    dragRef.current = null;
    if (rafRef.current !== 0) {
      globalThis.cancelAnimationFrame(rafRef.current);
      rafRef.current = 0;
    }
    const pendingY = pendingYRef.current;
    pendingYRef.current = null;
    if (pendingY !== null) paint(pendingY, false);
    if (!d) return;
    const last = d.samples.at(-1);
    const prev = d.samples.at(-2) ?? last;
    if (!last || !prev) return;
    const dt = last.t - prev.t;
    const vy = dt > 0 ? (last.y - prev.y) / dt : 0;
    const projected = yRef.current + (projectVelocity ? vy * PROJECTION_MS : 0);
    if (
      projectVelocity &&
      (vy > FLICK_DISMISS || projected > closedPxRef.current * 0.5)
    ) {
      dismiss();
      return;
    }
    paint(0, true);
  }, [dismiss, paint]);

  const [level, setLevel] = useState(0);
  useEffect(() => {
    if (!open) return;
    const { level: nextLevel, close } = markDetentSheetOpen();
    // Depth is only knowable after the imperative register.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setLevel(nextLevel);
    return close;
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") dismiss();
    };
    globalThis.addEventListener("keydown", onKey);
    return () => globalThis.removeEventListener("keydown", onKey);
  }, [open, dismiss]);

  if (!open) return null;

  const z = Z + Math.min(level, 24) * 2;
  const inset = `${String(OBSIDIAN_SHEET_INSET_PX)}px`;
  // Dock to the bottom. Keyboard lift sits outside the card; the home
  // indicator is padded inside. No side gap: a hairline inset around a
  // tall picker reads as an uneven frame.
  const bottom = `calc(${inset} + var(--kb-inset, 0px))`;

  return (
    <>
      <Box
        ref={scrimRef}
        aria-hidden
        data-obsidian-sheet-scrim="true"
        onPointerDown={dismissBackdrop.onPointerDown}
        onPointerMove={dismissBackdrop.onPointerMove}
        onPointerUp={dismissBackdrop.onPointerUp}
        onPointerCancel={dismissBackdrop.onPointerCancel}
        onClick={dismissBackdrop.onClick}
        sx={{
          position: "fixed",
          inset: 0,
          bgcolor: "common.black",
          zIndex: z,
          opacity: 0,
          touchAction: "none",
        }}
      />
      <Paper
        ref={sheetRef}
        role="dialog"
        aria-modal="true"
        aria-label={ariaLabel ?? (typeof title === "string" ? title : undefined)}
        data-detent-sheet="true"
        data-obsidian-sheet="true"
        elevation={0}
        onPointerDown={(event) => event.stopPropagation()}
        onClick={(event) => event.stopPropagation()}
        onTransitionEnd={(event) => {
          if (
            event.target === event.currentTarget &&
            event.propertyName === "transform"
          ) {
            delete event.currentTarget.dataset["detentMoving"];
            if (movingTimerRef.current !== null) {
              globalThis.clearTimeout(movingTimerRef.current);
              movingTimerRef.current = null;
            }
          }
        }}
        sx={{
          position: "fixed",
          left: inset,
          right: inset,
          bottom,
          maxHeight: `calc(${String(OBSIDIAN_SHEET_MAX_FRACTION * 100)}dvh - ${inset} - var(--kb-inset, 0px))`,
          zIndex: z + 1,
          willChange: "transform",
          contain: "layout paint",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          borderRadius: `${String(OBSIDIAN_SHEET_RADIUS_PX)}px ${String(OBSIDIAN_SHEET_RADIUS_PX)}px 0 0`,
          border: "none",
          bgcolor: "background.paper",
          backgroundImage: "none",
          boxShadow: (t) =>
            `0 8px 28px ${
              alpha(t.palette.common.black, t.palette.mode === "dark" ? 0.38 : 0.12)
            }`,
          outline: "none",
          transformOrigin: "center bottom",
          transform: "translate3d(0, 100dvh, 0) scale(0.96)",
        }}
      >
        <Box
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={() => finishDrag(true)}
          onPointerCancel={() => finishDrag(false)}
          onLostPointerCapture={() => finishDrag(false)}
          sx={{
            flexShrink: 0,
            touchAction: "none",
            cursor: "grab",
            userSelect: "none",
            pt: 0.5,
            pb: title == null ? 0 : 0,
          }}
        >
          <Box
            sx={{
              display: "flex",
              justifyContent: "center",
              alignItems: "center",
              height: HANDLE_HEIGHT,
            }}
          >
            <Box
              sx={{
                width: 36,
                height: 4,
                borderRadius: 2,
                bgcolor: "text.disabled",
                opacity: 0.55,
              }}
            />
          </Box>
          {title == null ? null : (
            <Typography
              variant="subtitle1"
              sx={{ fontWeight: 700, px: 2.25, pb: 0.5, pt: 0.125 }}
            >
              {title}
            </Typography>
          )}
        </Box>
        <Box
          sx={{
            flex: "0 1 auto",
            minHeight: 0,
            overflowY: "auto",
            WebkitOverflowScrolling: "touch",
            overscrollBehavior: "contain",
            px: 2.25,
            pb: actions == null
              ? floatingDismiss
                ? `calc(76px + ${SAFE_INSIDE})`
                : SAFE_INSIDE
              : 0.75,
            "[data-detent-moving] &": { transform: "none" },
          }}
        >
          {children}
        </Box>
        {actions == null && floatingDismiss
          ? (
            <Box
              sx={{
                position: "absolute",
                left: 0,
                right: 0,
                bottom: 0,
                zIndex: 2,
                px: 2,
                pb: SAFE_INSIDE,
                pointerEvents: "none",
                "& > *": { pointerEvents: "auto" },
              }}
            >
              <MobileSheetDismiss onClose={dismiss} />
            </Box>
          )
          : null}
        {actions == null ? null : (
          <Box
            sx={{
              flexShrink: 0,
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              flexWrap: "nowrap",
              gap: 1,
              px: 2.25,
              pt: 1,
              pb: SAFE_INSIDE,
              borderTop: 1,
              borderColor: "divider",
            }}
          >
            {actions}
          </Box>
        )}
      </Paper>
    </>
  );
}
