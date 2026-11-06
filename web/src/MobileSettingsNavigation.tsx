import { Box, Typography } from "@mui/material";
import { MobileSheetActionGroup } from "@cowboy/app-shell";
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
      width: "100%", display: "grid", gridTemplateColumns: section ? "106px minmax(0, 1fr) 106px" : "56px minmax(0, 1fr) 56px",
      alignItems: "center", gap: 1, position: "relative", isolation: "isolate", pointerEvents: "none",
    }}>
      <Box data-settings-navigation-actions>
      <MobileSheetActionGroup actions={[
        ...(section ? [{ key: "back", label: `Back to Settings from ${title}`, onPress: onBack, icon: <ArrowBackIosNew /> }] : []),
        { key: "close", label: "Close settings", onPress: onClose, icon: <Close /> },
      ]} />
      </Box>
      <Typography title={title} noWrap sx={{ minWidth: 0, textAlign: "center", fontSize: "1.125rem", lineHeight: 1.3, fontWeight: 600 }}>{title}</Typography>
    </Box>
  );
}
