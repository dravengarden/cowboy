/** The top of the Machine browser panel, like a browser's: whose browser on
 * which Machine (small), and an address field. The field never shows the
 * remote page's URL (Cowboy cannot read it); it keeps the last address sent
 * from this panel as its placeholder. Submitting navigates the shown browser
 * (`addressCommand`); a refused address is an inline message that does not
 * move the frame. */
import {
  CloseFullscreenRounded,
  CloseRounded,
  LockOutlined,
  OpenInFullRounded,
} from "@mui/icons-material";
import { Box, IconButton, InputBase, Paper, Stack, Tooltip, Typography } from "@mui/material";
import { type ReactNode, useState } from "react";

export function BrowserAddressField({
  enabled,
  lastUrl,
  touch,
  onSubmit,
}: {
  enabled: boolean;
  lastUrl: string | undefined;
  touch: boolean;
  /** Returns the message to show when the address is refused. */
  onSubmit: (text: string) => string | undefined;
}): React.JSX.Element {
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | undefined>(undefined);
  const placeholder = enabled ? (lastUrl ?? "Enter an https address") : "Open the browser first";
  return (
    <Box
      component="form"
      role="search"
      aria-label="Browser address"
      onSubmit={(event: React.FormEvent<HTMLFormElement>) => {
        event.preventDefault();
        const refused = onSubmit(value);
        setError(refused);
        if (refused) return;
        setValue("");
        (event.currentTarget.querySelector("input") as HTMLInputElement | null)?.blur();
      }}
      // A press on the field must not start a sheet drag.
      onPointerDown={(event: React.PointerEvent) => event.stopPropagation()}
      sx={{ position: "relative", flex: 1, minWidth: 0 }}
    >
      <Stack
        direction="row"
        alignItems="center"
        spacing={0.75}
        sx={{
          height: touch ? 40 : 34,
          px: 1.25,
          borderRadius: touch ? 999 : 1.5,
          bgcolor: "action.hover",
          border: 1,
          borderColor: error ? "error.main" : "transparent",
          opacity: enabled ? 1 : 0.7,
          "&:focus-within": { borderColor: error ? "error.main" : "primary.main" },
        }}
      >
        <LockOutlined sx={{ fontSize: "1rem", color: "text.secondary" }} />
        <InputBase
          value={value}
          disabled={!enabled}
          placeholder={placeholder}
          onChange={(event) => {
            setValue(event.target.value);
            setError(undefined);
          }}
          onKeyDown={(event) => {
            if (event.key !== "Escape") return;
            event.stopPropagation();
            setValue("");
            setError(undefined);
            event.currentTarget.querySelector("input")?.blur();
          }}
          inputProps={{
            "aria-label": "Address",
            "aria-invalid": error ? true : undefined,
            "aria-describedby": error ? "machine-browser-address-error" : undefined,
            inputMode: "url",
            enterKeyHint: "go",
            autoCapitalize: "off",
            autoCorrect: "off",
            autoComplete: "off",
            spellCheck: false,
          }}
          sx={{ flex: 1, minWidth: 0, fontSize: touch ? 16 : 14 }}
        />
      </Stack>
      {error && (
        <Paper
          id="machine-browser-address-error"
          role="alert"
          elevation={6}
          sx={{
            position: "absolute",
            top: "calc(100% + 6px)",
            left: 0,
            right: 0,
            zIndex: 3,
            px: 1.5,
            py: 1,
            color: "error.main",
          }}
        >
          <Typography variant="body2">{error}</Typography>
        </Paper>
      )}
    </Box>
  );
}

function Identity({
  accountLabel,
  machineLabel,
  status,
  align,
}: {
  accountLabel: string;
  machineLabel: string;
  status: string;
  align: "left" | "center";
}): React.JSX.Element {
  return (
    <Typography
      variant="caption"
      color="text.secondary"
      noWrap
      title={`${accountLabel} · ${machineLabel} · ${status}`}
      sx={{ display: "block", minWidth: 0, textAlign: align, lineHeight: 1.4 }}
    >
      <Box component="strong" sx={{ color: "text.primary", fontWeight: 600 }}>
        {accountLabel}
      </Box>
      {` · ${machineLabel} · ${status}`}
    </Typography>
  );
}

/** Phone: identity line above the address field, Close at the leading edge,
 * clearing the status bar and side safe areas. */
export function MobileBrowserTopBar({
  accountLabel,
  machineLabel,
  status,
  address,
  onClose,
}: {
  accountLabel: string;
  machineLabel: string;
  status: string;
  address: ReactNode;
  onClose: () => void;
}): React.JSX.Element {
  return (
    <Box
      data-machine-browser-topbar
      sx={{
        flexShrink: 0,
        position: "relative",
        zIndex: 2,
        borderBottom: 1,
        borderColor: "divider",
        bgcolor: "background.paper",
        pt: "calc(var(--cowboy-system-top-clearance, env(safe-area-inset-top, 0px)) + 4px)",
        pb: 0.75,
        pl: "max(4px, env(safe-area-inset-left, 0px))",
        pr: "max(12px, env(safe-area-inset-right, 0px))",
      }}
    >
      <Box sx={{ px: 6 }}>
        <Identity
          accountLabel={accountLabel}
          machineLabel={machineLabel}
          status={status}
          align="center"
        />
      </Box>
      <Stack direction="row" alignItems="center" spacing={0.5}>
        <IconButton aria-label="Close browser" onClick={onClose} sx={{ width: 44, height: 44 }}>
          <CloseRounded />
        </IconButton>
        {address}
      </Stack>
    </Box>
  );
}

/** Desktop: one dense row with navigation, the address field, identity,
 * the menu, the full-screen toggle and Close. */
export function DesktopBrowserTopBar({
  accountLabel,
  machineLabel,
  status,
  navigation,
  address,
  menu,
  fullscreen,
  onToggleFullscreen,
  onClose,
}: {
  accountLabel: string;
  machineLabel: string;
  status: string;
  navigation: ReactNode;
  address: ReactNode;
  menu: ReactNode;
  fullscreen: boolean;
  onToggleFullscreen: () => void;
  onClose: () => void;
}): React.JSX.Element {
  const fullLabel = fullscreen ? "Exit full screen" : "Full screen";
  return (
    <Stack
      direction="row"
      alignItems="center"
      spacing={0.5}
      data-machine-browser-topbar
      sx={{ flexShrink: 0, position: "relative", zIndex: 2, px: 1, py: 0.75 }}
    >
      {navigation}
      {address}
      <Box sx={{ maxWidth: 320, minWidth: 0, px: 1 }}>
        <Identity
          accountLabel={accountLabel}
          machineLabel={machineLabel}
          status={status}
          align="left"
        />
      </Box>
      {menu}
      <Tooltip title={fullscreen ? "Exit full screen (Esc)" : "Full screen"}>
        <IconButton
          aria-label={fullLabel}
          aria-pressed={fullscreen}
          onClick={onToggleFullscreen}
          sx={{ width: 34, height: 34 }}
        >
          {fullscreen ? <CloseFullscreenRounded /> : <OpenInFullRounded />}
        </IconButton>
      </Tooltip>
      <Tooltip title="Close">
        <IconButton aria-label="Close browser" onClick={onClose} sx={{ width: 34, height: 34 }}>
          <CloseRounded />
        </IconButton>
      </Tooltip>
    </Stack>
  );
}
