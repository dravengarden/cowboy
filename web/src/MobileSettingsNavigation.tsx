import { Box, ButtonBase, IconButton, Typography } from "@mui/material";
import { ArrowBackIosNew, Close } from "@mui/icons-material";
import { FloatingActionIsland } from "@cowboy/app-shell";
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
    <Box data-mobile-settings-navigation sx={{ width: "100%", display: "flex", justifyContent: "flex-start", pointerEvents: "none" }}>
      <FloatingActionIsland columns="minmax(0, 1fr) 46px" maxWidth="max-content" minHeight={56}>
        {section
          ? <ButtonBase aria-label={`Back to Settings from ${title}`} onClick={onBack} sx={{ minHeight: 46, minWidth: 0, gap: 1, px: 1.5, borderRadius: "999px" }}>
              <ArrowBackIosNew sx={{ fontSize: "1.125rem", flexShrink: 0 }} />
              <Typography component="span" sx={{ fontSize: "0.875rem", fontWeight: 600, overflowWrap: "anywhere" }}>{title}</Typography>
            </ButtonBase>
          : <Typography sx={{ alignSelf: "center", px: 1.5, fontSize: "0.875rem", fontWeight: 600 }}>{title}</Typography>}
        <IconButton aria-label="Close settings" onClick={onClose} sx={{ width: 46, height: 46 }}><Close sx={{ fontSize: "1.25rem" }} /></IconButton>
      </FloatingActionIsland>
    </Box>
  );
}
