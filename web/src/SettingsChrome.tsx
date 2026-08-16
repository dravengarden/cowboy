import {
  ArticleOutlined,
  DnsOutlined,
  InfoOutlined,
  TuneOutlined,
} from "@mui/icons-material";
import { Box, ButtonBase, Stack, Typography } from "@mui/material";
import { alpha } from "@mui/material/styles";
import type { ReactNode } from "react";
import type { ControlCenterTab } from "./desktop/controlCenterTabs";
import type { SettingsProductFocus } from "./appSettings";

const DESTINATIONS: readonly {
  value: ControlCenterTab;
  label: string;
  icon: ReactNode;
}[] = [
  { value: "settings", label: "Settings", icon: <TuneOutlined fontSize="small" /> },
  { value: "machines", label: "Machines", icon: <DnsOutlined fontSize="small" /> },
  { value: "info", label: "Info", icon: <InfoOutlined fontSize="small" /> },
  { value: "logs", label: "Logs", icon: <ArticleOutlined fontSize="small" /> },
];

const PRODUCTS: readonly { value: SettingsProductFocus; label: string }[] = [
  { value: "agent", label: "Agent" },
  { value: "code", label: "Code" },
];

export function settingsDestinationLabel(tab: ControlCenterTab): string {
  return DESTINATIONS.find((item) => item.value === tab)?.label ?? "Settings";
}

/** Icon rail for the four Settings destinations. Replaces a cramped 4-way pill. */
export function SettingsDestinationRail({
  value,
  onChange,
}: {
  value: ControlCenterTab;
  onChange: (value: ControlCenterTab) => void;
}): React.JSX.Element {
  return (
    <Box
      role="tablist"
      aria-label="Settings destinations"
      sx={{
        display: "grid",
        gridTemplateColumns: "repeat(4, minmax(0, 1fr))",
        gap: 0.25,
      }}
    >
      {DESTINATIONS.map((item) => {
        const selected = item.value === value;
        return (
          <ButtonBase
            key={item.value}
            role="tab"
            aria-selected={selected}
            onClick={(): void => onChange(item.value)}
            onPointerDown={(event): void => event.stopPropagation()}
            sx={{
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              gap: 0.5,
              py: 0.5,
              borderRadius: 2,
              color: selected ? "primary.main" : "text.secondary",
            }}
          >
            <Box
              aria-hidden
              sx={{
                width: 44,
                height: 32,
                borderRadius: 1.75,
                display: "grid",
                placeItems: "center",
                bgcolor: selected
                  ? (theme) =>
                    alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.2 : 0.12)
                  : "transparent",
              }}
            >
              {item.icon}
            </Box>
            <Typography
              variant="caption"
              sx={{
                fontWeight: selected ? 700 : 550,
                letterSpacing: 0.1,
                lineHeight: 1,
              }}
            >
              {item.label}
            </Typography>
          </ButtonBase>
        );
      })}
    </Box>
  );
}

/** Quiet Agent / Code switch. Not a second full-width pill. */
export function SettingsProductSwitch({
  value,
  onChange,
}: {
  value: SettingsProductFocus;
  onChange: (value: SettingsProductFocus) => void;
}): React.JSX.Element {
  return (
    <Stack
      role="tablist"
      aria-label="Settings product"
      direction="row"
      spacing={2}
      alignItems="center"
    >
      {PRODUCTS.map((item) => {
        const selected = item.value === value;
        return (
          <ButtonBase
            key={item.value}
            role="tab"
            aria-selected={selected}
            onClick={(): void => onChange(item.value)}
            onPointerDown={(event): void => event.stopPropagation()}
            sx={{
              pb: 0.4,
              color: selected ? "text.primary" : "text.secondary",
              borderBottom: "2px solid",
              borderColor: selected ? "primary.main" : "transparent",
            }}
          >
            <Typography
              variant="body2"
              sx={{ fontWeight: selected ? 720 : 550, lineHeight: 1.2 }}
            >
              {item.label}
            </Typography>
          </ButtonBase>
        );
      })}
    </Stack>
  );
}
