import DevicesRounded from "@mui/icons-material/DevicesRounded";
import { Alert, Box, Button, Stack, Typography } from "@mui/material";
import { useEffect, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { ProductSessionCapacityPanel } from "../auth/ProductSessionCapacityPanel";
import { ConfirmSheet } from "../Sheet";
import { useStoreSelector } from "../store";
import { useSurfaceProfile } from "../surface/SurfaceProfile";
import {
  productCapacityAlertHost,
  subscribeProductCapacityAlertHost,
} from "./productCapacityAlertHost";

function capacityMessage(
  status: "waiting" | "channel_limit" | "lost" | "unavailable",
  position?: number,
): { label: string; title: string; detail: string } {
  switch (status) {
    case "waiting":
      return {
        label: position ? `Waiting · #${position}` : "Waiting for a seat",
        title: "Active client limit reached",
        detail:
          "This view stays read-only while it waits fairly. Release one of your other active clients or leave this view in the queue.",
      };
    case "channel_limit":
      return {
        label: "Too many views",
        title: "Client channel limit reached",
        detail:
          "This logical client has too many open Cowboy views. Close a duplicate tab or window, then this view will reconnect automatically.",
      };
    case "lost":
      return {
        label: "Seat released",
        title: "This active seat was released",
        detail:
          "Another signed-in view reclaimed this client. Cowboy is reconnecting and will wait for the next available seat.",
      };
    case "unavailable":
      return {
        label: "Capacity unavailable",
        title: "Capacity service unavailable",
        detail:
          "Cowboy closed the mutating connection because it could not safely verify the active-client lease. It will retry automatically.",
      };
  }
}

export function ProductActiveCapacityGuard(): React.JSX.Element | null {
  const capacity = useStoreSelector((state) => state.activeCapacity);
  const surface = useSurfaceProfile();
  const mobile = surface.kind !== "desktop";
  const desktopHost = useSyncExternalStore(
    subscribeProductCapacityAlertHost,
    productCapacityAlertHost,
    productCapacityAlertHost,
  );
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (!capacity || capacity.status === "active") setOpen(false);
  }, [capacity]);

  if (!capacity || capacity.status === "active") return null;
  const content = capacityMessage(capacity.status, capacity.position);
  const urgent = capacity.status !== "waiting";
  const reminder = (
    <Button
      data-product-capacity-alert-button
      data-desktop-topbar-action={!mobile ? "capacity" : undefined}
      aria-label={`${content.label}. Open active-client management`}
      title={content.title}
      variant="outlined"
      color={urgent ? "error" : "warning"}
      size="small"
      startIcon={<DevicesRounded fontSize="small" />}
      onClick={() => setOpen(true)}
      sx={{
        pointerEvents: "auto",
        maxWidth: mobile ? "min(17rem, calc(100vw - 24px))" : 172,
        minHeight: mobile ? 44 : undefined,
        px: mobile ? 1.5 : 0.75,
        borderRadius: mobile ? 999 : undefined,
        bgcolor: "background.paper",
        boxShadow: mobile ? 8 : "none",
        textTransform: "none",
        whiteSpace: "nowrap",
        "&:hover": { bgcolor: "background.paper" },
      }}
    >
      <Typography variant="caption" fontWeight={800} noWrap>
        {content.label}
      </Typography>
    </Button>
  );
  const reminderSurface = open
    ? null
    : mobile
    ? (
      <Box
        sx={{
          position: "fixed",
          top:
            "calc(max(env(safe-area-inset-top, 0px), var(--cowboy-system-top-clearance, 0px)) + 60px)",
          right: 12,
          zIndex: (theme) => theme.zIndex.tooltip + 1,
          pointerEvents: "none",
        }}
      >
        {reminder}
      </Box>
    )
    : desktopHost
    ? createPortal(reminder, desktopHost)
    : (
      <Box
        sx={{
          position: "fixed",
          top: 12,
          right: 16,
          zIndex: (theme) => theme.zIndex.tooltip + 1,
          pointerEvents: "none",
        }}
      >
        {reminder}
      </Box>
    );

  return (
    <>
      {reminderSurface}
      <ConfirmSheet
        open={open}
        onClose={() => setOpen(false)}
        title={content.title}
        wide
        actions={
          <Button onClick={() => setOpen(false)} sx={{ minHeight: 44 }}>
            Close
          </Button>
        }
      >
        <Stack spacing={2} sx={{ px: 2, py: 1.5, maxHeight: "70vh", overflowY: "auto" }}>
          <Alert severity={urgent ? "error" : "warning"}>{content.detail}</Alert>
          <ProductSessionCapacityPanel />
        </Stack>
      </ConfirmSheet>
    </>
  );
}
