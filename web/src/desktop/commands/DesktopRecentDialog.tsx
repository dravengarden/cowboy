import { ScrollEdges } from "@cowboy/app-shell";
import { useEffect, useMemo, useRef, useState } from "react";
import { alpha, Box, ButtonBase, InputBase, Stack, Typography } from "@mui/material";
import {
  ChatBubbleOutline,
  DescriptionOutlined,
  History,
  Search,
} from "@mui/icons-material";
import { desktopSize } from "../../surface/desktopSize";
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

/** Multi-token case-insensitive filter over recent items' title and detail. */
export function filterRecentItems(
  items: readonly DesktopRecentItem[],
  query: string,
): DesktopRecentItem[] {
  const tokens = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return [...items];
  return items.filter((item) => {
    const haystack = `${item.title} ${item.detail ?? ""} ${item.kind}`.toLowerCase();
    return tokens.every((token) => haystack.includes(token));
  });
}

/** Safe index stepping that never produces `-1` on an empty list. */
export function stepRecentIndex(
  count: number,
  current: number,
  action: "next" | "prev" | "first" | "last",
): number {
  if (count <= 0) return 0;
  const last = count - 1;
  if (action === "first") return 0;
  if (action === "last") return last;
  if (action === "next") return current >= last ? 0 : current + 1;
  return current <= 0 ? last : current - 1;
}

/**
 * Recent (`␣O`): the jump list of Sessions and Drafts this device opened,
 * newest first (FOCUS.md "Recent"). It opens on the previous item, so
 * `␣O` `Enter` equals `␣⇥`. Digits open a row at once; `J/K` move, `/` filters,
 * `L` or `Enter` open, `H` or `Esc` close.
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
}): React.JSX.Element | null {
  if (!open) return null;
  return <DesktopRecentDialogContent items={items} onClose={onClose} onOpen={onOpen} />;
}

function handleRecentDialogKeyDown(
  event: React.KeyboardEvent<HTMLDivElement>,
  count: number,
  selected: number,
  setSelected: React.Dispatch<React.SetStateAction<number>>,
  choose: (index: number) => void,
  onClose: () => void,
  onStartFilter: () => void,
  pendingGo: React.MutableRefObject<number>,
): void {
  const native = event.nativeEvent;
  if (desktopImeOwnsKey(native) || event.metaKey || event.ctrlKey || event.altKey) return;
  const digit = desktopRecentDigit(event.code);
  const key = workspaceCommandKey(native);
  const now = Date.now();
  const armed = pendingGo.current > 0 && now - pendingGo.current <= 1200;
  if (armed) {
    if (["Shift", "Control", "Meta", "Alt", "AltGraph"].includes(key)) return;
    pendingGo.current = 0;
    event.preventDefault();
    event.stopPropagation();
    if (!event.repeat && key === "g" && count > 0) setSelected(0);
    return;
  }
  pendingGo.current = 0;
  if (key === "g") {
    event.preventDefault();
    event.stopPropagation();
    if (!event.repeat) pendingGo.current = now;
    return;
  }
  let handled = true;
  if (key === "/") onStartFilter();
  else if (key === "h") onClose();
  else if (count <= 0) {
    handled = digit !== null ||
      key === "j" || key === "k" || key === "ArrowDown" || key === "ArrowUp" ||
      key === "Home" || key === "End" ||
      key === "Tab" || key === "G" || key === "l" || key === "Enter";
  } else if (digit !== null) choose(digit - 1);
  else if (key === "j" || key === "ArrowDown" || (key === "Tab" && !event.shiftKey)) {
    setSelected((index) => stepRecentIndex(count, index, "next"));
  } else if (key === "k" || key === "ArrowUp" || key === "Tab") {
    setSelected((index) => stepRecentIndex(count, index, "prev"));
  } else if (key === "Home") setSelected(0);
  else if (key === "End" || key === "G") setSelected(stepRecentIndex(count, selected, "last"));
  else if (key === "l" || key === "Enter") choose(selected);
  else handled = false;
  if (handled) {
    event.preventDefault();
    event.stopPropagation();
  }
}

function RecentFilterBar({
  inputRef,
  query,
  count,
  selected,
  onQueryChange,
  onStep,
  onChoose,
  onCancel,
}: {
  inputRef: React.RefObject<HTMLInputElement | null>;
  query: string;
  count: number;
  selected: number;
  onQueryChange: (value: string) => void;
  onStep: (action: "next" | "prev") => void;
  onChoose: (index: number) => void;
  onCancel: () => void;
}): React.JSX.Element {
  return (
    <Box sx={{ px: 1.5, pt: 1, pb: 0.5 }}>
      <Box
        sx={{
          minHeight: desktopSize(36),
          px: 1.25,
          display: "flex",
          alignItems: "center",
          gap: 1,
          border: 1,
          borderColor: (theme) => alpha(theme.palette.primary.main, 0.3),
          borderRadius: `${DESKTOP_INSET_RADIUS}px`,
          bgcolor: (theme) => alpha(theme.palette.background.paper, 0.52),
          "&:focus-within": {
            borderColor: "primary.main",
            bgcolor: "background.paper",
          },
        }}
      >
        <Search aria-hidden sx={{ fontSize: "1rem", color: "text.secondary" }} />
        <InputBase
          inputRef={inputRef}
          fullWidth
          value={query}
          onChange={(event): void => onQueryChange(event.target.value)}
          onKeyDown={(event): void => {
            if (desktopImeOwnsKey(event.nativeEvent)) return;
            const ctrlNav = event.ctrlKey && !event.metaKey && !event.altKey;
            if (event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              onCancel();
            } else if (
              event.key === "ArrowDown" ||
              (ctrlNav && (event.code === "KeyJ" || event.code === "KeyN"))
            ) {
              event.preventDefault();
              if (count > 0) onStep("next");
            } else if (
              event.key === "ArrowUp" ||
              (ctrlNav && (event.code === "KeyK" || event.code === "KeyP"))
            ) {
              event.preventDefault();
              if (count > 0) onStep("prev");
            } else if (event.key === "Enter") {
              event.preventDefault();
              onChoose(selected);
            }
          }}
          placeholder="Filter recent Sessions and Drafts…"
          inputProps={{
            "aria-label": "Filter recent items",
            "data-desktop-escape": "close",
            "data-desktop-vim": "off",
          }}
          sx={{ fontSize: "0.84rem" }}
        />
      </Box>
    </Box>
  );
}

function RecentRow({
  item,
  index,
  active,
  now,
  onPointerHover,
  onSelect,
}: {
  item: DesktopRecentItem;
  index: number;
  active: boolean;
  now: number;
  onPointerHover: (x: number, y: number, index: number) => void;
  onSelect: (index: number) => void;
}): React.JSX.Element {
  return (
    <ButtonBase
      id={`desktop-recent-${String(index)}`}
      role="option"
      aria-selected={active}
      tabIndex={-1}
      data-desktop-recent-index={index}
      data-desktop-recent-key={item.key}
      onMouseMove={(event): void =>
        onPointerHover(event.clientX, event.clientY, index)}
      onClick={(): void => onSelect(index)}
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
}

function DesktopRecentDialogContent({
  items,
  onClose,
  onOpen,
}: {
  items: readonly DesktopRecentItem[];
  onClose: () => void;
  onOpen: (key: string) => void;
}): React.JSX.Element {
  const [selected, setSelected] = useState(0);
  const [filtering, setFiltering] = useState(false);
  const [query, setQuery] = useState("");
  const [now] = useState(() => Date.now());
  const listRef = useRef<HTMLDivElement>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const lastPointer = useRef<{ x: number; y: number } | null>(null);
  const pendingGo = useRef(0);
  const visible = useMemo(() => filterRecentItems(items, query), [items, query]);

  useEffect(() => {
    const frame = requestAnimationFrame(() =>
      listRef.current?.focus({ preventScroll: true })
    );
    return () => cancelAnimationFrame(frame);
  }, []);
  useEffect(() => {
    setSelected(0);
  }, [query]);
  useEffect(() => {
    if (selected >= visible.length) {
      setSelected(Math.max(0, visible.length - 1));
    }
  }, [visible.length, selected]);
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(
      `[data-desktop-recent-index="${String(selected)}"]`,
    )?.scrollIntoView({ block: "nearest" });
  }, [selected, visible.length]);

  const startFilter = (): void => {
    setFiltering(true);
    requestAnimationFrame(() => filterRef.current?.focus());
  };
  const stopFilter = (): void => {
    setQuery("");
    setFiltering(false);
    requestAnimationFrame(() => listRef.current?.focus({ preventScroll: true }));
  };
  const choose = (index: number): void => {
    const item = visible[index];
    if (!item) return;
    onClose();
    onOpen(item.key);
  };
  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>): void => {
    if (document.activeElement === filterRef.current) return;
    handleRecentDialogKeyDown(
      event,
      visible.length,
      selected,
      setSelected,
      choose,
      onClose,
      startFilter,
      pendingGo,
    );
  };
  return (
    <DesktopModal
      open
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
        {
          label: "Navigate",
          slots: [
            { shortcut: "J/K", label: "Row" },
            { shortcut: "/", label: "Filter" },
          ],
        },
        { slots: [{ shortcut: "H/Esc", label: "Close" }] },
      ]}
    >
      {filtering && (
        <RecentFilterBar
          inputRef={filterRef}
          query={query}
          count={visible.length}
          selected={selected}
          onQueryChange={setQuery}
          onStep={(action) =>
            setSelected((index) => stepRecentIndex(visible.length, index, action))}
          onChoose={choose}
          onCancel={stopFilter}
        />
      )}
      <Box
        ref={listRef}
        role="listbox"
        aria-label="Recent sessions and drafts"
        aria-activedescendant={visible[selected]
          ? `desktop-recent-${String(selected)}`
          : undefined}
        tabIndex={-1}
        data-desktop-recent
        sx={{ px: 1, pb: 1, outline: "none", overflowY: "auto" }}
      >
        {visible.length === 0 && (
          <Typography color="text.secondary" sx={{ px: 1.5, py: 3 }}>
            {items.length === 0
              ? "Open another Session or Draft and it will appear here."
              : `No recent items match “${query.trim()}”.`}
          </Typography>
        )}
        {visible.map((item, index) => (
          <RecentRow
            key={item.key}
            item={item}
            index={index}
            active={index === selected}
            now={now}
            onPointerHover={(x, y, rowIndex): void => {
              const prev = lastPointer.current;
              if (prev && prev.x === x && prev.y === y) return;
              lastPointer.current = { x, y };
              setSelected(rowIndex);
            }}
            onSelect={choose}
          />
        ))}
      <ScrollEdges /></Box>
    </DesktopModal>
  );
}
