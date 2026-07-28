import { Close, PushPinOutlined } from "@mui/icons-material";
import {
  Box,
  IconButton,
  Menu,
  MenuItem,
  Stack,
  Typography,
} from "@mui/material";
import { useRef, useState } from "react";
import { navigationHaptic } from "../../haptic";
import { type ReviewTab, reviewTabKey } from "./reviewTabs";

function basename(path: string): string {
  return path.split("/").at(-1) || path;
}

function tabLabel(tab: ReviewTab): string {
  return basename(tab.path);
}

export function ReviewTabStrip({
  tabs,
  activeKey,
  showCloseButtons,
  onActivate,
  onClose,
  onCloseOthers,
  onTogglePin,
  allowCloseActions = true,
  allowReorder = true,
  onReorder,
}: {
  tabs: ReviewTab[];
  activeKey: string | undefined;
  showCloseButtons: boolean;
  onActivate: (tab: ReviewTab) => void;
  onClose: (key: string) => void;
  onCloseOthers: (key: string) => void;
  onTogglePin: (key: string) => void;
  allowCloseActions?: boolean;
  allowReorder?: boolean;
  onReorder?: (movingKey: string, targetKey: string) => void;
}): React.JSX.Element | null {
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
  const [menu, setMenu] = useState<{
    anchor: HTMLElement;
    tab: ReviewTab;
  }>();
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
      openMenu && current?.dragging && !current.moved
    ) {
      setMenu({ anchor: current.anchor, tab: current.tab });
    }
  };

  return (
    <>
      <Stack
        direction="row"
        data-review-tab-strip
        data-mobile-pager-ignore
        sx={{
          height: 42,
          minHeight: 42,
          maxHeight: 42,
          overflowX: "auto",
          overflowY: "hidden",
          touchAction: "pan-x",
          overscrollBehaviorY: "none",
          scrollbarWidth: "none",
          borderBottom: 1,
          borderColor: "divider",
          "&::-webkit-scrollbar": { display: "none" },
        }}
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
              onClick={() => {
                if (suppressClick.current) {
                  suppressClick.current = false;
                  return;
                }
                onActivate(tab);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
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
                  onActivate(tab);
                }
                finishPress(true);
              }}
              onPointerCancel={() => finishPress(false)}
              sx={{
                appearance: "none",
                flex: "0 0 auto",
                display: "flex",
                alignItems: "center",
                gap: 0.5,
                minWidth: 92,
                maxWidth: 190,
                height: 42,
                minHeight: 42,
                maxHeight: 42,
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
              {tab.pinned && <PushPinOutlined sx={{ fontSize: 14 }} />}
              <Typography variant="caption" noWrap sx={{ flex: 1 }}>
                {tabLabel(tab)}
              </Typography>
              {showCloseButtons && allowCloseActions && (
                <IconButton
                  aria-label={`Close ${tabLabel(tab)}`}
                  size="small"
                  onClick={(event) => {
                    event.stopPropagation();
                    onClose(key);
                  }}
                  sx={{
                    width: 32,
                    height: 32,
                    minWidth: 32,
                    minHeight: 32,
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
          sx={{ flex: "1 0 72px", minHeight: 42 }}
        />
      </Stack>
      <Menu
        anchorEl={menu?.anchor}
        open={menu !== undefined}
        onClose={() => setMenu(undefined)}
      >
        <MenuItem
          onClick={() => {
            if (menu) onTogglePin(reviewTabKey(menu.tab));
            setMenu(undefined);
          }}
        >
          {menu?.tab.pinned ? "Unpin" : "Pin"}
        </MenuItem>
        {allowCloseActions && (
          <MenuItem
            onClick={() => {
              if (menu) onCloseOthers(reviewTabKey(menu.tab));
              setMenu(undefined);
            }}
          >
            Close others
          </MenuItem>
        )}
        {allowCloseActions && (
          <MenuItem
            onClick={() => {
              if (menu) onClose(reviewTabKey(menu.tab));
              setMenu(undefined);
            }}
          >
            Close
          </MenuItem>
        )}
      </Menu>
    </>
  );
}
