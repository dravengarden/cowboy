import { alpha, Box, IconButton, Typography } from "@mui/material";
import { ArrowBackIosNew, Close } from "@mui/icons-material";
import type { MobileSettingsSection } from "./SettingsShell";

const titles: Record<MobileSettingsSection, string> = {
  appearance: "Appearance", reading: "Reading layout", agent: "Agent behavior", code: "Code & diff",
  notifications: "Notifications", providers: "Accounts & sign-in", machines: "Machines",
  info: "About", logs: "Logs", account: "Account",
};

export function MobileSettingsNavigation({ section, onBack, onClose }: {
  section: MobileSettingsSection | null;
  onBack: () => void;
  onClose: () => void;
}) {
  const title = section ? titles[section] : "Settings";
  return (
    <Box component="nav" aria-label="Settings navigation" data-mobile-settings-navigation sx={{
      width: "100%", display: "grid", gridTemplateColumns: section ? "90px minmax(0, 1fr) 90px" : "46px minmax(0, 1fr) 46px",
      alignItems: "center", gap: 1, position: "relative", isolation: "isolate", pointerEvents: "none",
      "& .MuiIconButton-root": { padding: 0, "& .MuiSvgIcon-root": { fontSize: "1.25rem" } },
    }}>
      <Box data-settings-navigation-actions sx={{ display: "flex", pointerEvents: "auto", borderRadius: "999px", bgcolor: "background.paper",
          border: "1px solid", borderColor: theme => alpha(theme.palette.common.white, theme.palette.mode === "dark" ? 0.15 : 0.7),
          boxShadow: theme => `0 3px 12px ${alpha(theme.palette.common.black, theme.palette.mode === "dark" ? 0.24 : 0.12)}` }}>
      <IconButton aria-label={section ? `Back to Settings from ${title}` : "Close settings"} onClick={section ? onBack : onClose}
        sx={{ width: 44, height: 44 }}>
        {section ? <ArrowBackIosNew /> : <Close />}
      </IconButton>
      {section && <IconButton aria-label="Close settings" onClick={onClose} sx={{ width: 44, height: 44, color: "text.secondary" }}><Close /></IconButton>}
      </Box>
      <Typography title={title} noWrap sx={{ minWidth: 0, textAlign: "center", fontSize: "1.125rem", lineHeight: 1.3, fontWeight: 600 }}>{title}</Typography>
    </Box>
  );
}
