import { ScrollEdges } from "@cowboy/app-shell";
import { Search } from "@mui/icons-material";
import {
  alpha,
  Box,
  InputBase,
  List,
  ListItemButton,
  ListItemText,
  Typography,
} from "@mui/material";
import { useEffect, useMemo, useRef, useState } from "react";
import { desktopSize } from "../../surface/desktopSize";
import { DESKTOP_INSET_RADIUS } from "../DesktopEmbeddedControl";
import { DesktopModal } from "../DesktopModal";
import {
  type DesktopCommand,
  useDesktopCommands,
} from "./DesktopCommandProvider";
import { DesktopShortcut } from "./DesktopKeycap";
import {
  filterPaletteCommands,
  firstSelectablePaletteIndex,
  loadRecentPaletteCommands,
  type PaletteCommandEntry,
  recordRecentPaletteCommand,
  saveRecentPaletteCommands,
  stepPaletteSelection,
} from "./commandPaletteModel";
import { desktopImeOwnsKey } from "./imeShortcut";
import { DESKTOP_SHORTCUTS } from "./workspaceShortcuts";

function paletteShortcutGroups(hasResults: boolean, selectedAvailable: boolean) {
  return [
    { slots: [{ shortcut: DESKTOP_SHORTCUTS.commands, label: "Palette", availability: "active" as const }] },
    {
      label: "Navigate",
      slots: [
        { shortcut: "↑/↓ · Ctrl+J/K", label: "Move", availability: hasResults ? "available" as const : "inactive" as const },
        { shortcut: "Enter", label: "Run", availability: selectedAvailable ? "available" as const : "inactive" as const },
      ],
    },
    { slots: [{ shortcut: "Esc", label: "Close" }] },
  ];
}

export function DesktopCommandPalette({
  onClose,
}: {
  onClose: () => void;
}): React.JSX.Element {
  // Mounted only while open, so subscribing to live command updates keeps
  // dynamic rows (such as approvals) current without waking DesktopCommandHost.
  const registry = useDesktopCommands();
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [recentIds, setRecentIds] = useState<string[]>(() =>
    loadRecentPaletteCommands()
  );
  const paletteInputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const frame = requestAnimationFrame(() => paletteInputRef.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, []);
  const entries = useMemo(
    () => filterPaletteCommands(registry.commands, query, recentIds),
    [registry.commands, query, recentIds],
  );
  // An explicit pick runs only the exact registered command instance this row
  // presented: if an approval prompt changed before activation, reject it.
  const runCommand = (command: DesktopCommand): void => {
    const live = registry.list().find((entry) => entry.id === command.id);
    if (live !== command) return;
    if (registry.execute(command.id, { anyScope: true })) {
      const nextRecent = recordRecentPaletteCommand(recentIds, command.id);
      setRecentIds(nextRecent);
      saveRecentPaletteCommands(nextRecent);
      onClose();
    }
  };
  useEffect(() => {
    setSelected(firstSelectablePaletteIndex(entries));
    // Re-anchor selection whenever the search query changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);
  useEffect(() => {
    if (selected >= entries.length) {
      setSelected(Math.max(0, entries.length - 1));
    }
  }, [entries.length, selected]);

  const runSelected = (): void => {
    const entry = entries[selected];
    if (entry && entry.available) runCommand(entry.command);
  };
  const selectedEntry = entries[selected];
  const selectedAvailable = selectedEntry?.available === true;

  return (
    <DesktopModal
      open
      onClose={onClose}
      title="Command Palette"
      description="Search and run every registered Desktop command."
      width={680}
      shortcutGroups={paletteShortcutGroups(entries.length > 0, selectedAvailable)}
    >
      <Box sx={{ px: 1.5, pb: 1.5, pt: 1.25 }}>
        <Box
          sx={{
            minHeight: desktopSize(44),
            px: 1.5,
            display: "flex",
            alignItems: "center",
            gap: 1.25,
            border: 1,
            borderColor: (theme) => alpha(theme.palette.primary.main, 0.3),
            borderRadius: `${DESKTOP_INSET_RADIUS}px`,
            bgcolor: (theme) => alpha(theme.palette.background.paper, 0.52),
            transition: "border-color 120ms ease, box-shadow 120ms ease, background-color 120ms ease",
            "&:focus-within": {
              borderColor: "primary.main",
              bgcolor: "background.paper",
              boxShadow: (theme) =>
                `0 0 0 2px ${alpha(theme.palette.primary.main, 0.16)}`,
            },
          }}
        >
          <Search
            aria-hidden
            sx={{ flexShrink: 0, color: "text.secondary", fontSize: "1.2rem" }}
          />
          <InputBase
            inputRef={paletteInputRef}
            autoFocus
            fullWidth
            value={query}
            onChange={(event): void => setQuery(event.target.value)}
            onKeyDown={(event): void => {
              if (desktopImeOwnsKey(event.nativeEvent)) return;
              const ctrlNav = event.ctrlKey && !event.metaKey && !event.altKey;
              if (
                event.key === "ArrowDown" ||
                (ctrlNav && (event.code === "KeyJ" || event.code === "KeyN"))
              ) {
                event.preventDefault();
                setSelected((value) => stepPaletteSelection(entries, value, 1));
              } else if (
                event.key === "ArrowUp" ||
                (ctrlNav && (event.code === "KeyK" || event.code === "KeyP"))
              ) {
                event.preventDefault();
                setSelected((value) => stepPaletteSelection(entries, value, -1));
              } else if (event.key === "Enter") {
                event.preventDefault();
                runSelected();
              }
            }}
            placeholder="Search commands…"
            // A launcher: typing is all it is for, so one Esc closes it
            // (FOCUS.md "Modals"), as in every command palette.
            inputProps={{
              "aria-label": "Search commands",
              "data-desktop-escape": "close",
              "data-desktop-vim": "off",
            }}
            sx={{
              minWidth: 0,
              fontSize: "0.9rem",
              "& .MuiInputBase-input": {
                p: 0,
                height: "1.5em",
                lineHeight: 1.5,
                caretColor: "primary.main",
                "&::placeholder": { color: "text.secondary", opacity: 0.72 },
              },
            }}
          />
        </Box>
        <DesktopCommandPaletteList
          entries={entries}
          query={query}
          selected={selected}
          onHover={setSelected}
          onRun={runCommand}
        />
      </Box>
    </DesktopModal>
  );
}

function DesktopCommandPaletteList({
  entries,
  query,
  selected,
  onHover,
  onRun,
}: {
  entries: readonly PaletteCommandEntry[];
  query: string;
  selected: number;
  onHover: (index: number) => void;
  onRun: (command: DesktopCommand) => void;
}): React.JSX.Element {
  const listRef = useRef<HTMLUListElement>(null);
  const lastPointer = useRef<{ x: number; y: number } | null>(null);
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(
      `[data-palette-index="${String(selected)}"]`,
    )?.scrollIntoView({ block: "nearest" });
  }, [selected, entries.length]);

  const showRecentHeaders = query.trim().length === 0 &&
    entries.some((entry) => entry.available && entry.recent);
  const firstNonRecentIndex = showRecentHeaders
    ? entries.findIndex((entry) => !entry.available || !entry.recent)
    : -1;

  return (
    <List
      ref={listRef}
      dense
      sx={{ maxHeight: "min(52vh, 460px)", overflowY: "auto", pt: 1 }}
    >
      {entries.length === 0 && (
        <Typography
          variant="body2"
          color="text.secondary"
          data-palette-empty
          sx={{ px: 1.5, py: 3, textAlign: "center" }}
        >
          {query.trim()
            ? `No commands match “${query.trim()}”.`
            : "No commands available."}
        </Typography>
      )}
      {entries.map(({ command, available, recent }, index) => (
        <Box key={command.id}>
          {showRecentHeaders && index === 0 && (
            <Typography
              variant="overline"
              color="primary.main"
              data-palette-section="recent"
              sx={{ display: "block", px: 1.5, pt: 0.25, pb: 0.25, lineHeight: 1.4, fontWeight: 700 }}
            >
              Recent
            </Typography>
          )}
          {showRecentHeaders && index === firstNonRecentIndex && (
            <Typography
              variant="overline"
              color="text.secondary"
              data-palette-section="all"
              sx={{ display: "block", px: 1.5, pt: 0.75, pb: 0.25, lineHeight: 1.4, fontWeight: 700 }}
            >
              All commands
            </Typography>
          )}
          <ListItemButton
            data-palette-index={index}
            data-palette-recent={recent ? "true" : undefined}
            selected={available && index === selected}
            disabled={!available}
            onMouseMove={(event): void => {
              if (!available) return;
              const prev = lastPointer.current;
              if (prev && prev.x === event.clientX && prev.y === event.clientY) {
                return;
              }
              lastPointer.current = { x: event.clientX, y: event.clientY };
              onHover(index);
            }}
            onClick={(): void => onRun(command)}
            sx={{ borderRadius: 1.5 }}
          >
            <ListItemText
              primary={command.title}
              secondary={!available
                ? (typeof command.disabledReason === "function"
                  ? command.disabledReason()
                  : command.disabledReason ?? "Unavailable")
                : command.description ?? command.id}
            />
            {(command.shortcut || command.sequence) && (
              <DesktopShortcut
                shortcut={command.shortcut ?? command.sequence?.join(" → ") ?? ""}
                availability={!available ? "inactive" : "available"}
              />
            )}
          </ListItemButton>
        </Box>
      ))}
    <ScrollEdges /></List>
  );
}
