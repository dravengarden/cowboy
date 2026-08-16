import CheckIcon from "@mui/icons-material/Check";
import { Box, Button, CircularProgress } from "@mui/material";
import { useEffect, useState } from "react";
import type { ConnectionStore } from "../_shell";
import {
  fetchReadyCowboyVersion,
  mobileUpdateBannerLabel,
} from "./mobileUpdateVersion";

/**
 * Mobile owns update activation explicitly. A foreground service-worker check
 * may discover a deploy while the user is reading or composing; silently
 * replacing that page is much more disruptive on a phone than on Desktop.
 * Keep the update visible and let the user choose the safe reload point.
 */
export function MobileConnectionBanner(
  { store }: { readonly store: ConnectionStore },
): React.JSX.Element | null {
  const banner = store.useConnectionBanner();
  const isUpdate = banner?.kind === "update";
  const [readyVersion, setReadyVersion] = useState<string>();
  useEffect(() => {
    if (!isUpdate) {
      setReadyVersion(undefined);
      return undefined;
    }
    let cancelled = false;
    void (async (): Promise<void> => {
      const registration = "serviceWorker" in navigator
        ? await navigator.serviceWorker.getRegistration()
        : undefined;
      const version = await fetchReadyCowboyVersion(
        undefined,
        registration?.waiting?.scriptURL,
      );
      if (!cancelled) setReadyVersion(version);
    })();
    return (): void => {
      cancelled = true;
    };
  }, [isUpdate]);
  if (!banner) return null;
  const palette = banner.kind === "down"
    ? "warning"
    : banner.kind === "reconnected"
    ? "success"
    : "info";
  const label = banner.kind === "down"
    ? "Connection lost — reconnecting…"
    : banner.kind === "reconnected"
    ? "Reconnected"
    : mobileUpdateBannerLabel(readyVersion);

  return (
    <Box
      role="status"
      aria-live="polite"
      sx={{
        position: "absolute",
        top: 0,
        left: 0,
        right: 0,
        width: "100%",
        maxWidth: "100%",
        boxSizing: "border-box",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        flexWrap: "wrap",
        gap: 1,
        px: 2,
        py: 0.75,
        pt: "calc(env(safe-area-inset-top, 0px) + 6px)",
        bgcolor: `${palette}.main`,
        color: `${palette}.contrastText`,
        fontSize: "0.8125rem",
        fontWeight: 600,
        zIndex: (theme) => theme.zIndex.tooltip + 1,
      }}
    >
      {banner.kind === "down" && (
        <CircularProgress size={14} color="inherit" thickness={5} />
      )}
      {banner.kind === "reconnected" && <CheckIcon sx={{ fontSize: "1.125rem" }} />}
      <span>{label}</span>
      {isUpdate && (
        <Button
          color="inherit"
          size="small"
          variant="outlined"
          onClick={() => void store.applyUpdate()}
          sx={{
            ml: 0.5,
            minHeight: 32,
            borderColor: "currentColor",
            borderRadius: 999,
            fontSize: "inherit",
            fontWeight: 700,
            textTransform: "none",
          }}
        >
          Update
        </Button>
      )}
    </Box>
  );
}
