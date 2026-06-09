import { useEffect, useSyncExternalStore } from "react";
import { Box, IconButton, Stack, Typography } from "@mui/material";
import Close from "@mui/icons-material/Close";
import ChevronLeft from "@mui/icons-material/ChevronLeft";
import ChevronRight from "@mui/icons-material/ChevronRight";
import InsertDriveFileOutlined from "@mui/icons-material/InsertDriveFileOutlined";
import type { Attachment } from "./attachments";

// Full-screen preview for an already-staged/pasted resource. A single module-
// level store drives ONE overlay (rendered once at the app root) that any
// thumbnail anywhere — the composer's staged strip OR a parked draft / queued
// row — opens by calling `openLightbox(list, index)`. Keeping the state global
// (not per-row) avoids prop-drilling a preview callback through PendingPanel →
// PendingRow → the chips, and guarantees a single overlay instance.

type LightboxState = { items: Attachment[]; index: number } | null;

let current: LightboxState = null;
const listeners = new Set<() => void>();
const emit = (): void => {
  for (const l of listeners) l();
};

/** Open the preview on `items[index]`. No-op for an empty list. */
export function openLightbox(items: Attachment[], index: number): void {
  if (items.length === 0) return;
  current = { items, index: Math.max(0, Math.min(index, items.length - 1)) };
  emit();
}

function close(): void {
  current = null;
  emit();
}

function step(delta: number): void {
  if (!current) return;
  const n = current.items.length;
  current = { items: current.items, index: (current.index + delta + n) % n };
  emit();
}

const NAV_BTN = {
  color: "#fff",
  bgcolor: "rgba(255,255,255,0.12)",
  "&:hover": { bgcolor: "rgba(255,255,255,0.22)" },
} as const;

export function ResourceLightbox(): React.JSX.Element | null {
  const state = useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    () => current,
    () => null,
  );

  // Desktop keys: Esc closes, ←/→ page through a multi-attachment set.
  useEffect(() => {
    if (!state) return undefined;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") close();
      else if (e.key === "ArrowLeft") step(-1);
      else if (e.key === "ArrowRight") step(1);
    };
    globalThis.addEventListener("keydown", onKey);
    return () => globalThis.removeEventListener("keydown", onKey);
  }, [state]);

  if (!state) return null;
  const a = state.items[state.index];
  if (!a) return null;
  const multi = state.items.length > 1;

  return (
    // Tap the backdrop to dismiss; the media itself stops propagation so a tap
    // ON the image doesn't close it. position:fixed covers the (cowboy) viewport.
    <Box
      onClick={close}
      sx={{
        position: "fixed",
        inset: 0,
        zIndex: 2000,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        bgcolor: "rgba(0,0,0,0.92)",
        backdropFilter: "blur(8px)",
        WebkitBackdropFilter: "blur(8px)",
      }}
    >
      <IconButton
        aria-label="close preview"
        onClick={(e): void => {
          e.stopPropagation();
          close();
        }}
        sx={{
          position: "absolute",
          top: "max(env(safe-area-inset-top), 12px)",
          right: "max(env(safe-area-inset-right), 12px)",
          ...NAV_BTN,
        }}
      >
        <Close />
      </IconButton>

      {a.isImage && a.previewUrl
        ? (
          <Box
            component="img"
            src={a.previewUrl}
            alt={a.name}
            onClick={(e): void => e.stopPropagation()}
            sx={{
              maxWidth: "92vw",
              maxHeight: "82vh",
              objectFit: "contain",
              borderRadius: 1.5,
              boxShadow: "0 16px 56px rgba(0,0,0,0.6)",
            }}
          />
        )
        : (
          // Non-image (a file resource) can't be rendered — show its identity.
          <Stack
            onClick={(e): void => e.stopPropagation()}
            alignItems="center"
            spacing={1.5}
            sx={{ color: "#fff", px: 4, textAlign: "center" }}
          >
            <InsertDriveFileOutlined sx={{ fontSize: 72, opacity: 0.7 }} />
            <Typography sx={{ wordBreak: "break-word", maxWidth: 320 }}>
              {a.name}
            </Typography>
            <Typography variant="caption" sx={{ opacity: 0.6 }}>
              {a.mimeType}
            </Typography>
          </Stack>
        )}

      {/* Caption + (for a set) a position counter, pinned above the home edge. */}
      <Typography
        variant="caption"
        sx={{
          position: "absolute",
          bottom: "max(env(safe-area-inset-bottom), 16px)",
          left: 0,
          right: 0,
          textAlign: "center",
          color: "rgba(255,255,255,0.78)",
          px: 6,
          wordBreak: "break-word",
        }}
      >
        {a.name}
        {multi
          ? `  ·  ${String(state.index + 1)} / ${String(state.items.length)}`
          : ""}
      </Typography>

      {multi && (
        <>
          <IconButton
            aria-label="previous"
            onClick={(e): void => {
              e.stopPropagation();
              step(-1);
            }}
            sx={{
              position: "absolute",
              left: "max(env(safe-area-inset-left), 8px)",
              ...NAV_BTN,
            }}
          >
            <ChevronLeft />
          </IconButton>
          <IconButton
            aria-label="next"
            onClick={(e): void => {
              e.stopPropagation();
              step(1);
            }}
            sx={{
              position: "absolute",
              right: "max(env(safe-area-inset-right), 8px)",
              ...NAV_BTN,
            }}
          >
            <ChevronRight />
          </IconButton>
        </>
      )}
    </Box>
  );
}
