import { Box, IconButton, Typography } from "@mui/material";
import { FloatingActionIsland } from "@cowboy/app-shell";
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
      width: "100%", display: "grid", gridTemplateColumns: section ? "102px minmax(0, 1fr) 102px" : "54px minmax(0, 1fr) 54px",
      alignItems: "center", gap: 1, position: "relative", isolation: "isolate", pointerEvents: "none",
      "& .MuiIconButton-root": { padding: 0, "& .MuiSvgIcon-root": { fontSize: "1.25rem" } },
    }}>
      <Box data-settings-navigation-actions>
      <FloatingActionIsland columns={section ? "44px 44px" : "44px"} maxWidth={section ? 102 : 54}>
      <IconButton aria-label={section ? `Back to Settings from ${title}` : "Close settings"} onClick={section ? onBack : onClose}
        sx={{ width: 44, height: 44 }}>
        {section ? <ArrowBackIosNew /> : <Close />}
      </IconButton>
      {section && <IconButton aria-label="Close settings" onClick={onClose} sx={{ width: 44, height: 44, color: "text.secondary" }}><Close /></IconButton>}
      </FloatingActionIsland>
      </Box>
      <Typography title={title} noWrap sx={{ minWidth: 0, textAlign: "center", fontSize: "1.125rem", lineHeight: 1.3, fontWeight: 600 }}>{title}</Typography>
    </Box>
  );
}
