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
import { reviewTabKey, type ReviewTab } from "./reviewTabs";

function basename(path: string): string {
  return path.split("/").at(-1) || path;
}

export function ReviewTabStrip({
  tabs,
  activeKey,
  onActivate,
  onClose,
  onCloseOthers,
  onTogglePin,
}: {
  tabs: ReviewTab[];
  activeKey: string | undefined;
  onActivate: (tab: ReviewTab) => void;
  onClose: (key: string) => void;
  onCloseOthers: (key: string) => void;
  onTogglePin: (key: string) => void;
}): React.JSX.Element | null {
  const timer = useRef<number | undefined>(undefined);
  const [menu, setMenu] = useState<{
    anchor: HTMLElement;
    tab: ReviewTab;
  }>();
  if (tabs.length === 0) return null;

  const cancelLongPress = (): void => {
    if (timer.current !== undefined) globalThis.clearTimeout(timer.current);
    timer.current = undefined;
  };

  return (
    <>
      <Stack
        direction="row"
        data-mobile-pager-ignore
        sx={{
          minHeight: 42,
          overflowX: "auto",
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
              component="button"
              type="button"
              onClick={() => onActivate(tab)}
              onPointerDown={(event) => {
                cancelLongPress();
                const anchor = event.currentTarget;
                timer.current = globalThis.setTimeout(() => {
                  setMenu({ anchor, tab });
                  timer.current = undefined;
                }, 450);
              }}
              onPointerUp={cancelLongPress}
              onPointerCancel={cancelLongPress}
              onPointerLeave={cancelLongPress}
              sx={{
                appearance: "none",
                flex: "0 0 auto",
                display: "flex",
                alignItems: "center",
                gap: 0.5,
                minWidth: 92,
                maxWidth: 190,
                minHeight: 42,
                pl: 1.5,
                pr: 0.25,
                border: 0,
                borderRight: 1,
                borderColor: "divider",
                borderBottom: 2,
                borderBottomColor: active ? "primary.main" : "transparent",
                bgcolor: active ? "action.selected" : "transparent",
                color: "text.primary",
              }}
            >
              {tab.pinned && <PushPinOutlined sx={{ fontSize: 14 }} />}
              <Typography variant="caption" noWrap sx={{ flex: 1 }}>
                {basename(tab.path)}
              </Typography>
              <IconButton
                aria-label={`Close ${basename(tab.path)}`}
                size="small"
                onClick={(event) => {
                  event.stopPropagation();
                  onClose(key);
                }}
                sx={{ minWidth: 32, minHeight: 32 }}
              >
                <Close sx={{ fontSize: 16 }} />
              </IconButton>
            </Box>
          );
        })}
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
        <MenuItem
          onClick={() => {
            if (menu) onCloseOthers(reviewTabKey(menu.tab));
            setMenu(undefined);
          }}
        >
          Close others
        </MenuItem>
        <MenuItem
          onClick={() => {
            if (menu) onClose(reviewTabKey(menu.tab));
            setMenu(undefined);
          }}
        >
          Close
        </MenuItem>
      </Menu>
    </>
  );
}
