import { useEffect, useRef, useState } from "react";
import { alpha, Box, ButtonBase, Stack, Typography } from "@mui/material";
import {
  ChatBubbleOutline,
  DescriptionOutlined,
  History,
} from "@mui/icons-material";
import { ProviderIcon } from "../../ProviderIcon";
import { DesktopModal } from "../DesktopModal";
import { DESKTOP_INSET_RADIUS } from "../DesktopEmbeddedControl";
import type { DesktopRecentItem } from "../sessionVisits";
import { DesktopKeycap } from "./DesktopKeycap";
import { desktopImeOwnsKey } from "./imeShortcut";
import { workspaceCommandKey } from "./workspaceCommandKey";

/** `now`, `5m`, `3h`, `2d`: how long ago the item was last opened. */
export function desktopRecentAge(at: number, now: number): string {
  const minutes = Math.floor(Math.max(0, now - at) / 60_000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${String(minutes)}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${String(hours)}h`;
  return `${String(Math.floor(hours / 24))}d`;
}

/** The digit a physical key selects (`1`…`9`, top row or keypad), else null. */
export function desktopRecentDigit(code: string): number | null {
  const match = /^(?:Digit|Numpad)([1-9])$/.exec(code);
  return match ? Number(match[1]) : null;
}

/**
 * Recent (`␣O`): the jump list of Sessions and Drafts this device opened,
 * newest first (FOCUS.md "Recent"). It opens on the previous item, so
 * `␣O` `Enter` equals `␣⇥`. Digits open a row at once; `J/K` move, `L` or
 * `Enter` open, `H` or `Esc` close.
 */
export function DesktopRecentDialog({
  open,
  items,
  onClose,
  onOpen,
}: {
  open: boolean;
  items: readonly DesktopRecentItem[];
  onClose: () => void;
  onOpen: (key: string) => void;
}): React.JSX.Element {
  const [selected, setSelected] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  const listRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return undefined;
    setSelected(0);
    setNow(Date.now());
    const frame = requestAnimationFrame(() =>
      listRef.current?.focus({ preventScroll: true })
    );
    return () => cancelAnimationFrame(frame);
  }, [open]);
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(
      `[data-desktop-recent-index="${String(selected)}"]`,
    )?.scrollIntoView({ block: "nearest" });
  }, [selected]);
  const choose = (index: number): void => {
    const item = items[index];
    if (!item) return;
    onClose();
    onOpen(item.key);
  };
  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>): void => {
    const native = event.nativeEvent;
    if (
      desktopImeOwnsKey(native) || event.metaKey || event.ctrlKey ||
      event.altKey
    ) return;
    const digit = desktopRecentDigit(event.code);
    const key = workspaceCommandKey(native);
    const last = items.length - 1;
    let handled = true;
    if (digit !== null) choose(digit - 1);
    else if (key === "j" || key === "ArrowDown" || (key === "Tab" && !event.shiftKey)) {
      setSelected((index) => index >= last ? 0 : index + 1);
    } else if (key === "k" || key === "ArrowUp" || key === "Tab") {
      setSelected((index) => index <= 0 ? last : index - 1);
    } else if (key === "g") setSelected(0);
    else if (key === "G") setSelected(last);
    else if (key === "l" || key === "Enter") choose(selected);
    else if (key === "h") onClose();
    else handled = false;
    if (handled) {
      event.preventDefault();
      event.stopPropagation();
    }
  };
  return (
    <DesktopModal
      open={open}
      onClose={onClose}
      title="Recent"
      description="Sessions and Drafts you opened on this device, newest first."
      icon={<History color="primary" />}
      width={560}
      onShortcutKeyDown={onKeyDown}
      shortcutGroups={[
        {
          label: "Open",
          slots: [
            { shortcut: "1…9", label: "Row" },
            { shortcut: "L/Enter", label: "Selected" },
          ],
        },
        { label: "Move", slots: [{ shortcut: "J/K", label: "Row" }] },
        { slots: [{ shortcut: "H/Esc", label: "Close" }] },
      ]}
    >
      <Box
        ref={listRef}
        role="listbox"
        aria-label="Recent sessions and drafts"
        aria-activedescendant={items[selected]
          ? `desktop-recent-${String(selected)}`
          : undefined}
        tabIndex={-1}
        data-desktop-recent
        sx={{ px: 1, pb: 1, outline: "none", overflowY: "auto" }}
      >
        {items.length === 0 && (
          <Typography color="text.secondary" sx={{ px: 1.5, py: 3 }}>
            Open another Session or Draft and it will appear here.
          </Typography>
        )}
        {items.map((item, index) => {
          const active = index === selected;
          return (
            <ButtonBase
              key={item.key}
              id={`desktop-recent-${String(index)}`}
              role="option"
              aria-selected={active}
              tabIndex={-1}
              data-desktop-recent-index={index}
              data-desktop-recent-key={item.key}
              onMouseMove={(): void => setSelected(index)}
              onClick={(): void => choose(index)}
              sx={{
                width: "100%",
                justifyContent: "flex-start",
                gap: 1.25,
                px: 1.25,
                py: 0.9,
                borderRadius: `${DESKTOP_INSET_RADIUS}px`,
                textAlign: "left",
                bgcolor: (theme) =>
                  active ? alpha(theme.palette.primary.main, 0.1) : "transparent",
                outline: (theme) =>
                  active ? `1px solid ${alpha(theme.palette.primary.main, 0.35)}` : "none",
              }}
            >
              {index < 9
                ? <DesktopKeycap keyLabel={String(index + 1)} accent={active} />
                : <Box sx={{ width: "1.5rem" }} />}
              <Box
                component="span"
                sx={{ display: "inline-flex", color: "text.secondary", fontSize: "1.15rem" }}
              >
                {item.kind === "draft"
                  ? <DescriptionOutlined fontSize="inherit" />
                  : item.provider
                  ? <ProviderIcon provider={item.provider} fontSize="inherit" />
                  : <ChatBubbleOutline fontSize="inherit" />}
              </Box>
              <Stack sx={{ minWidth: 0, flex: 1 }}>
                <Typography variant="body2" fontWeight={active ? 700 : 560} noWrap>
                  {item.title}
                </Typography>
                {item.detail && (
                  <Typography variant="caption" color="text.secondary" noWrap>
                    {item.detail}
                  </Typography>
                )}
              </Stack>
              {item.status === "busy" && (
                <Box
                  component="span"
                  aria-label="Working"
                  sx={{
                    width: 6,
                    height: 6,
                    borderRadius: "50%",
                    bgcolor: "warning.main",
                    flexShrink: 0,
                  }}
                />
              )}
              <Typography variant="caption" color="text.secondary" sx={{ flexShrink: 0 }}>
                {desktopRecentAge(item.at, now)}
              </Typography>
            </ButtonBase>
          );
        })}
      </Box>
    </DesktopModal>
  );
}
