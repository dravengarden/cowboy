import {
  AccountCircleOutlined,
  ArticleOutlined,
  ChevronRight,
  DnsOutlined,
  InfoOutlined,
} from "@mui/icons-material";
import { Box, ButtonBase, Stack, Typography } from "@mui/material";
import type { ReactNode } from "react";
import type { ControlCenterTab } from "./desktop/controlCenterTabs";

const DESTINATION_LABELS: Record<ControlCenterTab, string> = {
  settings: "Settings",
  providers: "Providers",
  machines: "Machines",
  info: "About",
  logs: "Logs",
};

export function settingsDestinationLabel(tab: ControlCenterTab): string {
  return DESTINATION_LABELS[tab];
}

export function SettingsNavRow({
  icon,
  label,
  onPress,
}: {
  icon: ReactNode;
  label: string;
  onPress: () => void;
}): React.JSX.Element {
  return (
    <ButtonBase
      onClick={onPress}
      onPointerDown={(event): void => event.stopPropagation()}
      sx={{
        width: "100%",
        minHeight: 48,
        px: 0.25,
        borderRadius: 1.5,
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        textAlign: "left",
        "&:active": { bgcolor: "action.hover" },
      }}
    >
      <Stack direction="row" spacing={1.25} alignItems="center" sx={{ minWidth: 0 }}>
        <Box
          aria-hidden
          sx={{
            width: 28,
            height: 28,
            display: "grid",
            placeItems: "center",
            color: "text.secondary",
          }}
        >
          {icon}
        </Box>
        <Typography variant="body2" sx={{ fontWeight: 600 }}>
          {label}
        </Typography>
      </Stack>
      <ChevronRight fontSize="small" sx={{ color: "text.disabled" }} />
    </ButtonBase>
  );
}

export const SETTINGS_PROVIDER_ROW = {
  tab: "providers" as const,
  label: "Accounts & sign-in",
  icon: <AccountCircleOutlined fontSize="small" />,
};

export const SETTINGS_MORE_ROWS = [
  { tab: "machines" as const, label: "Machines", icon: <DnsOutlined fontSize="small" /> },
  { tab: "info" as const, label: "About", icon: <InfoOutlined fontSize="small" /> },
  { tab: "logs" as const, label: "Logs", icon: <ArticleOutlined fontSize="small" /> },
];
