import { Close, PushPinOutlined } from "@mui/icons-material";
import { shortcutKey } from "../../desktop/commands/imeShortcut";
import {
  Box,
  IconButton,
  Menu,
  MenuItem,
  Stack,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useRef, useState } from "react";
import { navigationHaptic } from "../../haptic";
import { isPairedTouchClick } from "../../touchGestures";
import { type ReviewTab, reviewTabKey } from "./reviewTabs";

function ReviewTabOverflowEdges({
  overflow,
}: {
  overflow: { readonly left: boolean; readonly right: boolean };
}): React.JSX.Element {
  return (
    <>
      {overflow.left && (
        <Box
          aria-hidden
          data-review-tabs-overflow="left"
          sx={{
            position: "absolute",
            top: 0,
            bottom: 1,
            left: 0,
            width: 20,
            pointerEvents: "none",
            background: (theme) =>
              `linear-gradient(to right, ${theme.palette.background.default}, transparent)`,
          }}
        />
      )}
      {overflow.right && (
        <Box
          aria-hidden
          data-review-tabs-overflow="right"
          sx={{
            position: "absolute",
            top: 0,
            bottom: 1,
            right: 0,
            width: 24,
            pointerEvents: "none",
            background: (theme) =>
              `linear-gradient(to left, ${theme.palette.background.default}, transparent)`,
          }}
        />
      )}
    </>
  );
}

function basename(path: string): string {
  return path.split("/").at(-1) || path;
}

function tabLabel(tab: ReviewTab): string {
  return basename(tab.path);
}

function ReviewTabMenu({
  menu,
  tabs,
  allowPin,
  allowCloseActions,
  onCloseMenu,
  onTogglePin,
  onClose,
  onCloseOthers,
  onCloseAll,
}: {
  menu: { anchor: HTMLElement; tab: ReviewTab } | undefined;
  tabs: readonly ReviewTab[];
  allowPin: boolean;
  allowCloseActions: boolean;
  onCloseMenu: () => void;
  onTogglePin: (key: string) => void;
  onClose: (key: string, anchor: HTMLElement) => void;
  onCloseOthers: (key: string, anchor: HTMLElement) => void;
  onCloseAll: (anchor: HTMLElement) => void;
}): React.JSX.Element {
  return (
    <Menu anchorEl={menu?.anchor} open={menu !== undefined} onClose={onCloseMenu}>
      {allowPin && (
        <MenuItem
          onClick={() => {
            if (menu) onTogglePin(reviewTabKey(menu.tab));
            onCloseMenu();
          }}
        >
          {menu?.tab.pinned ? "Unpin" : "Pin"}
        </MenuItem>
      )}
      {allowCloseActions && (
        <MenuItem
          onClick={() => {
            if (menu) onClose(reviewTabKey(menu.tab), menu.anchor);
            onCloseMenu();
          }}
        >
          Close
        </MenuItem>
      )}
      {allowCloseActions && (
        <MenuItem
          disabled={menu !== undefined &&
            tabs.every((tab) =>
              reviewTabKey(tab) === reviewTabKey(menu.tab) || tab.pinned
            )}
          onClick={() => {
            if (menu) onCloseOthers(reviewTabKey(menu.tab), menu.anchor);
            onCloseMenu();
          }}
        >
          Close others
        </MenuItem>
      )}
      {allowCloseActions && (
        <MenuItem
          onClick={() => {
            if (menu) onCloseAll(menu.anchor);
            onCloseMenu();
          }}
        >
          Close all
        </MenuItem>
      )}
    </Menu>
  );
}

const stripSx = {
  height: 44,
  minHeight: 44,
  maxHeight: 44,
  pl: "env(safe-area-inset-left, 0px)",
  pr: "env(safe-area-inset-right, 0px)",
  overflowX: "auto",
  overflowY: "hidden",
  touchAction: "pan-x",
  overscrollBehaviorY: "none",
  scrollbarWidth: "none",
  borderBottom: 1,
  borderColor: "divider",
  "&::-webkit-scrollbar": { display: "none" },
} as const;

export function ReviewTabStrip({
  tabs,
  activeKey,
  onActivate,
  onClose,
  onCloseOthers,
  onCloseAll,
  onTogglePin,
  allowCloseActions = true,
  allowReorder = true,
  allowPin = true,
  onReorder,
  beforeTouchActivate,
}: {
  tabs: ReviewTab[];
  activeKey: string | undefined;
  onActivate: (tab: ReviewTab) => void;
  onClose: (key: string, anchor: HTMLElement) => void;
  onCloseOthers: (key: string, anchor: HTMLElement) => void;
  onCloseAll: (anchor: HTMLElement) => void;
  onTogglePin: (key: string) => void;
  allowCloseActions?: boolean;
  allowReorder?: boolean;
  allowPin?: boolean;
  onReorder?: (movingKey: string, targetKey: string) => void;
  beforeTouchActivate?: () => void;
}): React.JSX.Element | null {
  const strip = useRef<HTMLDivElement>(null);
  const timer = useRef<number | undefined>(undefined);
  const suppressClick = useRef(false);
  const press = useRef<
    | {
      pointerId: number;
      tab: ReviewTab;
      anchor: HTMLElement;
      x: number;
      y: number;
      dragging: boolean;
      moved: boolean;
    }
    | undefined
  >(undefined);
  const [draggingKey, setDraggingKey] = useState<string>();
  const [overflow, setOverflow] = useState({ left: false, right: false });
  const [menu, setMenu] = useState<{
    anchor: HTMLElement;
    tab: ReviewTab;
  }>();
  const updateOverflow = useCallback((): void => {
    const el = strip.current;
    if (!el) return;
    const left = el.scrollLeft > 2;
    const right = el.scrollLeft + el.clientWidth < el.scrollWidth - 2;
    setOverflow((prev) =>
      prev.left === left && prev.right === right ? prev : { left, right }
    );
  }, []);
  const tabOrder = tabs.map(reviewTabKey).join("\0");
  useEffect(() => () => {
    if (timer.current !== undefined) globalThis.clearTimeout(timer.current);
  }, []);
  useEffect(() => {
    const el = strip.current;
    if (!el) return undefined;
    updateOverflow();
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(updateOverflow);
    observer.observe(el);
    return () => observer.disconnect();
  }, [tabOrder, updateOverflow]);
  useEffect(() => {
    const viewport = strip.current;
    if (!viewport || !activeKey) return;
    const active = [...viewport.querySelectorAll<HTMLElement>(
      "[data-review-tab-key]",
    )].find((tab) => tab.dataset.reviewTabKey === activeKey);
    if (!active) return;
    const padding = 8;
    const left = active.offsetLeft;
    const right = left + active.offsetWidth;
    const visibleLeft = viewport.scrollLeft;
    const visibleRight = visibleLeft + viewport.clientWidth;
    if (left < visibleLeft + padding) {
      viewport.scrollTo({
        left: Math.max(0, left - padding),
        behavior: "smooth",
      });
    } else if (right > visibleRight - padding) {
      viewport.scrollTo({
        left: right - viewport.clientWidth + padding,
        behavior: "smooth",
      });
    }
  }, [activeKey, tabOrder]);
  if (tabs.length === 0) return null;

  const cancelLongPress = (): void => {
    if (timer.current !== undefined) globalThis.clearTimeout(timer.current);
    timer.current = undefined;
  };
  const finishPress = (openMenu: boolean): void => {
    const current = press.current;
    cancelLongPress();
    press.current = undefined;
    setDraggingKey(undefined);
    if (
      openMenu && current?.dragging && !current.moved &&
      (allowPin || allowCloseActions)
    ) {
      setMenu({ anchor: current.anchor, tab: current.tab });
    }
  };

  return (
    <>
      <Box sx={{ position: "relative", flexShrink: 0, minWidth: 0 }}>
      <Stack
        ref={strip}
        direction="row"
        data-review-tab-strip
        data-mobile-pager-ignore
        onScroll={updateOverflow}
        sx={stripSx}
      >
        {tabs.map((tab) => {
          const key = reviewTabKey(tab);
          const active = key === activeKey;
          return (
            <Box
              key={key}
              data-review-tab-key={key}
              role="tab"
              tabIndex={0}
              aria-selected={active}
              onClick={(event) => {
                const suppress = isPairedTouchClick(
                  suppressClick.current,
                  event.detail,
                  "pointerType" in event.nativeEvent
                    ? String(event.nativeEvent.pointerType)
                    : "",
                );
                suppressClick.current = false;
                if (suppress) {
                  event.preventDefault();
                  return;
                }
                onActivate(tab);
              }}
              onKeyDown={(event) => {
                if (
                  ["Enter", " "].includes(shortcutKey(event.nativeEvent) ?? "")
                ) {
                  event.preventDefault();
                  onActivate(tab);
                }
              }}
              onPointerDown={(event) => {
                if (event.pointerType === "mouse") return;
                cancelLongPress();
                suppressClick.current = false;
                const anchor = event.currentTarget;
                anchor.setPointerCapture(event.pointerId);
                press.current = {
                  pointerId: event.pointerId,
                  tab,
                  anchor,
                  x: event.clientX,
                  y: event.clientY,
                  dragging: false,
                  moved: false,
                };
                timer.current = globalThis.setTimeout(() => {
                  if (!press.current) return;
                  suppressClick.current = true;
                  press.current.dragging = true;
                  setDraggingKey(key);
                  navigationHaptic();
                  timer.current = undefined;
                }, 380);
              }}
              onPointerMove={(event) => {
                const current = press.current;
                if (!current || current.pointerId !== event.pointerId) return;
                const deltaX = event.clientX - current.x;
                const deltaY = event.clientY - current.y;
                const distance = Math.hypot(deltaX, deltaY);
                if (!current.dragging) {
                  if (distance > 10) {
                    cancelLongPress();
                    suppressClick.current = true;
                    press.current = undefined;
                  }
                  return;
                }
                if (distance > 8) current.moved = true;
                if (
                  !allowReorder ||
                  Math.abs(deltaY) >= Math.abs(deltaX)
                ) return;
                const target = document
                  .elementFromPoint(event.clientX, event.clientY)
                  ?.closest<HTMLElement>("[data-review-tab-key]");
                const targetKey = target?.dataset.reviewTabKey;
                if (targetKey && targetKey !== key) {
                  onReorder?.(key, targetKey);
                }
              }}
              onPointerUp={() => {
                const current = press.current;
                if (current && !current.dragging && !current.moved) {
                  suppressClick.current = true;
                  // Activation can remove the strip and reveal a file row
                  // under this finger. Its owner guards the paired click.
                  beforeTouchActivate?.();
                  onActivate(tab);
                }
                finishPress(true);
              }}
              onPointerCancel={() => finishPress(false)}
              onLostPointerCapture={() => finishPress(false)}
              sx={{
                appearance: "none",
                flex: "0 0 auto",
                display: "flex",
                alignItems: "center",
                gap: 0.5,
                minWidth: 92,
                maxWidth: 190,
                height: 44,
                minHeight: 44,
                maxHeight: 44,
                boxSizing: "border-box",
                pl: 1.5,
                pr: 0.25,
                border: 0,
                borderRight: 1,
                borderColor: "divider",
                borderBottom: 2,
                borderBottomColor: active ? "primary.main" : "transparent",
                bgcolor: active ? "action.selected" : "transparent",
                color: "text.primary",
                opacity: draggingKey === key ? 0.68 : 1,
                transition: "opacity 120ms ease",
                touchAction: "pan-x",
                overscrollBehaviorY: "none",
                userSelect: "none",
                WebkitUserSelect: "none",
                WebkitTouchCallout: "none",
              }}
              onContextMenu={(event) => event.preventDefault()}
            >
              {tab.pinned && <PushPinOutlined sx={{ fontSize: "0.875rem" }} />}
              <Typography variant="caption" noWrap sx={{ flex: 1 }}>
                {tabLabel(tab)}
              </Typography>
              {active && allowCloseActions && (
                <IconButton
                  aria-label={`Close ${tabLabel(tab)}`}
                  size="small"
                  onClick={(event) => {
                    event.stopPropagation();
                    onClose(key, event.currentTarget);
                  }}
                  onPointerDown={(event) => event.stopPropagation()}
                  sx={{
                    width: 44,
                    height: 44,
                    minWidth: 44,
                    minHeight: 44,
                    p: 0,
                    flexShrink: 0,
                  }}
                >
                  <Close sx={{ fontSize: "1.1rem" }} />
                </IconButton>
              )}
            </Box>
          );
        })}
        <Box
          aria-hidden
          data-mobile-pager-allow
          sx={{ flex: "1 0 72px", minHeight: 44 }}
        />
      </Stack>
      <ReviewTabOverflowEdges overflow={overflow} />
      </Box>
      <ReviewTabMenu
        menu={menu}
        tabs={tabs}
        allowPin={allowPin}
        allowCloseActions={allowCloseActions}
        onCloseMenu={() => setMenu(undefined)}
        onTogglePin={onTogglePin}
        onClose={onClose}
        onCloseOthers={onCloseOthers}
        onCloseAll={onCloseAll}
      />
    </>
  );
}
