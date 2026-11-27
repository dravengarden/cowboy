import { Box, ButtonBase, Divider, ListItemText, Menu, MenuItem, type Theme } from "@mui/material";
import { useEffect, useId, useRef, useState } from "react";
import { HintTooltip } from "../HintTooltip";
import type { DesktopTabItem } from "./DesktopTabStrip";

export const DESKTOP_OPEN_TAB_OVERFLOW_EVENT = "cowboy:desktop-open-tab-overflow";

export function TabStripOverflowButton({
  overflow,
  onOpen,
  onCloseAllOverflow,
}: {
  overflow: readonly DesktopTabItem[];
  onOpen: (key: string) => void;
  onCloseAllOverflow: (keys: readonly string[]) => void;
}): React.JSX.Element | null {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const menuId = useId();

  useEffect(() => {
    const onRequest = (): void => {
      if (overflow.length > 0 && buttonRef.current) setAnchor(buttonRef.current);
    };
    globalThis.addEventListener(DESKTOP_OPEN_TAB_OVERFLOW_EVENT, onRequest);
    return () => globalThis.removeEventListener(DESKTOP_OPEN_TAB_OVERFLOW_EVENT, onRequest);
  }, [overflow.length]);

  useEffect(() => {
    if (overflow.length === 0 && anchor !== null) setAnchor(null);
  }, [anchor, overflow.length]);

  if (overflow.length === 0) return null;
  const open = anchor !== null;
  const unpinnedKeys = overflow.filter((item) => !item.pinned).map((item) => item.key);
  return (
    <>
      <HintTooltip
        title={`${String(overflow.length)} more open ${overflow.length === 1 ? "tab" : "tabs"}`}
        detail="Click or press Enter to switch or close overflow tabs"
        enterDelay={300}
      >
        <ButtonBase
          ref={buttonRef}
          type="button"
          tabIndex={0}
          aria-label={`Show ${String(overflow.length)} more open tabs`}
          aria-haspopup="menu"
          aria-expanded={open}
          aria-controls={open ? menuId : undefined}
          data-desktop-tabs-overflow-menu
          onClick={(event): void => setAnchor(event.currentTarget)}
          sx={{
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            flexShrink: 0,
            height: "1.625rem",
            px: "0.5rem",
            borderRadius: "6px",
            fontSize: "0.72rem",
            fontWeight: 650,
            fontVariantNumeric: "tabular-nums",
            color: "text.secondary",
            bgcolor: "action.hover",
            "&:hover": { color: "text.primary", bgcolor: "action.selected" },
            "&:focus-visible": { outline: "2px solid", outlineColor: "primary.main", outlineOffset: 1 },
          }}
        >
          +{String(overflow.length)}
        </ButtonBase>
      </HintTooltip>
      <Menu
        id={menuId}
        anchorEl={anchor}
        open={open}
        onClose={(): void => setAnchor(null)}
        slotProps={{ list: { dense: true, sx: { minWidth: 220, maxWidth: 320, maxHeight: 340 } } }}
      >
        {overflow.map((item) => (
          <MenuItem
            key={item.key}
            data-desktop-overflow-tab={item.key}
            onClick={(): void => {
              setAnchor(null);
              onOpen(item.key);
            }}
            sx={{ gap: 0.75 }}
          >
            <Box component="span" sx={{ display: "inline-flex", alignItems: "center", gap: 0.35, flexShrink: 0, "& [data-desktop-tab-icon]": { fontSize: "0.95rem" } }}>
              {item.leading}
            </Box>
            <ListItemText
              primary={item.title}
              slotProps={{ primary: { noWrap: true, sx: { fontSize: "0.8rem" } } }}
            />
          </MenuItem>
        ))}
        {unpinnedKeys.length > 0 && <Divider />}
        {unpinnedKeys.length > 0 && (
          <MenuItem
            data-desktop-close-overflow-tabs
            onClick={(): void => {
              setAnchor(null);
              onCloseAllOverflow(unpinnedKeys);
            }}
            sx={{ fontSize: "0.78rem", color: "text.secondary" }}
          >
            Close {String(unpinnedKeys.length)} overflow tabs
          </MenuItem>
        )}
      </Menu>
    </>
  );
}

export function TabStripOverflowEdges({
  overflow,
}: {
  overflow: { readonly left: boolean; readonly right: boolean };
}): React.JSX.Element {
  return (
    <>
      {overflow.left && (
        <Box aria-hidden data-desktop-tabs-overflow="left" sx={{ position: "absolute", top: 0, bottom: 1, left: 0, width: 22, pointerEvents: "none", background: (t: Theme) => `linear-gradient(to right, ${t.palette.background.default}, transparent)` }} />
      )}
      {overflow.right && (
        <Box aria-hidden data-desktop-tabs-overflow="right" sx={{ position: "absolute", top: 0, bottom: 1, right: "2.75rem", width: 24, pointerEvents: "none", background: (t: Theme) => `linear-gradient(to left, ${t.palette.background.default}, transparent)` }} />
      )}
    </>
  );
}
