import { useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  Box,
  IconButton,
  InputAdornment,
  Stack,
  TextField,
  Tooltip,
  Typography,
  useTheme,
} from "@mui/material";
import {
  AddCircleOutline,
  DragIndicator,
  RemoveCircleOutline,
  RestartAlt,
  Search,
} from "@mui/icons-material";
import { DetentSheet, MobileSheetDismiss } from "./components/app-shell";
import { COMPOSER_COMMANDS, COMPOSER_COMMANDS_BY_ID } from "./composerCommands";
import {
  DEFAULT_COMPOSER_TOOLBAR,
  setComposerToolbar,
  useComposerToolbar,
} from "./composerToolbarConfig";
import { haptic } from "./haptic";
import { releaseMobileComposerFocus } from "./composer/mobileComposerFocus";

// Obsidian's "Manage toolbar options": curate which commands appear in the
// fullscreen markdown toolbar and in what order. Search-to-add a disabled
// command, ⊖ to remove, drag the ≡ handle to reorder. Writes the per-device
// persisted config (composerToolbarConfig). Reorder is a hand-rolled pointer
// drag — `touch-action: none` on the handle separates drag from list scroll, and
// the LIVE preview order is committed once on drop (a per-move .set would thrash
// localStorage).
export function ComposerToolbarSettings({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}): React.JSX.Element {
  const theme = useTheme();
  const enabled = useComposerToolbar();
  const [query, setQuery] = useState("");
  const [drag, setDrag] = useState<{ id: string; overIndex: number } | null>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // This control opens a full-cover modal sheet, unlike the inline formatting
  // and attachment actions which deliberately preserve editor focus. End the
  // active editing session before the sheet paints so the compact Composer's
  // `:focus-within` geometry cannot outlive the keyboard that the sheet hides.
  // Without this, iOS leaves the editor as document.activeElement while closing
  // its keyboard, and the background card remains expanded behind/after the
  // sheet as a large empty canvas.
  useLayoutEffect(() => {
    if (!open) return;
    releaseMobileComposerFocus();
  }, [open]);

  // The order shown: the persisted list, with the in-flight dragged id floated to
  // its hover slot so rows rearrange live under the finger (no commit yet).
  const order = useMemo(() => {
    if (drag === null) return enabled;
    const without = enabled.filter((id) => id !== drag.id);
    const at = Math.max(0, Math.min(without.length, drag.overIndex));
    return [...without.slice(0, at), drag.id, ...without.slice(at)];
  }, [enabled, drag]);

  const addable = COMPOSER_COMMANDS.filter(
    (c) =>
      !enabled.includes(c.id) &&
      c.label.toLowerCase().includes(query.trim().toLowerCase()),
  );

  const add = (id: string): void => {
    haptic();
    setComposerToolbar([...enabled, id]);
    setQuery("");
  };
  const remove = (id: string): void => {
    haptic();
    setComposerToolbar(enabled.filter((x) => x !== id));
  };
  const reset = (): void => {
    haptic();
    setComposerToolbar([...DEFAULT_COMPOSER_TOOLBAR]);
  };

  const onHandleDown = (id: string) => (e: React.PointerEvent): void => {
    e.preventDefault();
    e.stopPropagation();
    haptic();
    (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
    setDrag({ id, overIndex: enabled.indexOf(id) });
  };
  const onHandleMove = (e: React.PointerEvent): void => {
    setDrag((d) => {
      if (d === null) return d;
      const box = listRef.current;
      if (!box) return d;
      const rows = Array.from(box.querySelectorAll<HTMLElement>("[data-row]"));
      let idx = rows.length - 1;
      for (const [i, row] of rows.entries()) {
        const r = row.getBoundingClientRect();
        if (e.clientY < r.top + r.height / 2) {
          idx = i;
          break;
        }
      }
      return idx === d.overIndex ? d : { id: d.id, overIndex: idx };
    });
  };
  const onHandleUp = (): void => {
    if (drag !== null) {
      haptic();
      setComposerToolbar(order);
    }
    setDrag(null);
  };

  return (
    <DetentSheet
      open={open}
      onClose={onClose}
      ariaLabel="Toolbar options"
      cover
      frosted
      surfaceColor={theme.palette.background.default}
      footer={<MobileSheetDismiss onClose={onClose} />}
      footerOverlay
      header={
        <Box sx={{ px: 1.5, pb: 0.5 }}>
          <Typography variant="subtitle2" sx={{ fontWeight: 600 }}>
            Toolbar
          </Typography>
        </Box>
      }
    >
      <Box sx={{ px: 1.5, pb: 2 }}>
        <Stack
          direction="row"
          alignItems="center"
          justifyContent="space-between"
          sx={{ mb: 1 }}
        >
          <Typography variant="overline" color="text.secondary">
            Manage toolbar options
          </Typography>
          <Tooltip title="Reset to default">
            <IconButton size="small" aria-label="reset toolbar" onClick={reset}>
              <RestartAlt fontSize="small" />
            </IconButton>
          </Tooltip>
        </Stack>

        <TextField
          fullWidth
          size="small"
          placeholder="Select a command…"
          value={query}
          onChange={(e): void => setQuery(e.target.value)}
          sx={{ mb: 1 }}
          slotProps={{
            input: {
              startAdornment: (
                <InputAdornment position="start">
                  <Search fontSize="small" />
                </InputAdornment>
              ),
            },
          }}
        />

        {/* Add list — disabled commands matching the query. */}
        {query.trim() !== "" && (
          <Stack sx={{ mb: 1 }}>
            {addable.length === 0
              ? (
                <Typography variant="body2" color="text.secondary" sx={{ px: 1, py: 0.5 }}>
                  No matching command
                </Typography>
              )
              : addable.map((c) => (
                <Stack
                  key={c.id}
                  direction="row"
                  alignItems="center"
                  spacing={1.5}
                  onClick={(): void => add(c.id)}
                  sx={{
                    px: 1,
                    py: 0.75,
                    borderRadius: 1.5,
                    cursor: "pointer",
                    "&:active": { bgcolor: "action.selected" },
                  }}
                >
                  <AddCircleOutline color="primary" fontSize="small" />
                  <Box sx={{ display: "flex", color: "text.secondary" }}>{c.icon}</Box>
                  <Typography variant="body2">{c.label}</Typography>
                </Stack>
              ))}
          </Stack>
        )}

        {/* Enabled list — remove + drag-reorder. */}
        <Box ref={listRef}>
          {order
            .map((id) => COMPOSER_COMMANDS_BY_ID[id])
            .filter((c): c is (typeof COMPOSER_COMMANDS)[number] => c !== undefined)
            .map((c) => {
              const dragging = drag?.id === c.id;
              return (
                <Stack
                  key={c.id}
                  data-row=""
                  direction="row"
                  alignItems="center"
                  spacing={1.5}
                  sx={{
                    px: 1,
                    py: 0.75,
                    borderRadius: 1.5,
                    bgcolor: dragging ? "action.selected" : "transparent",
                    boxShadow: dragging ? 3 : 0,
                    opacity: dragging ? 0.95 : 1,
                  }}
                >
                  <IconButton
                    size="small"
                    aria-label={`remove ${c.label}`}
                    onClick={(): void => remove(c.id)}
                  >
                    <RemoveCircleOutline color="error" fontSize="small" />
                  </IconButton>
                  <Box sx={{ display: "flex", color: "text.secondary" }}>{c.icon}</Box>
                  <Typography variant="body2" sx={{ flex: 1 }}>
                    {c.label}
                  </Typography>
                  <Box
                    aria-label={`reorder ${c.label}`}
                    onPointerDown={onHandleDown(c.id)}
                    onPointerMove={onHandleMove}
                    onPointerUp={onHandleUp}
                    onPointerCancel={onHandleUp}
                    sx={{
                      display: "flex",
                      alignItems: "center",
                      px: 0.5,
                      color: "text.disabled",
                      cursor: "grab",
                      touchAction: "none", // drag the handle, scroll the list body
                      "&:active": { cursor: "grabbing" },
                    }}
                  >
                    <DragIndicator fontSize="small" />
                  </Box>
                </Stack>
              );
            })}
        </Box>
      </Box>
    </DetentSheet>
  );
}
