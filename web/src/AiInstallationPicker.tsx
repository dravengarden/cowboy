import {
  Box,
  ListItemIcon,
  MenuItem,
  TextField,
  Typography,
} from "@mui/material";
import type { ProviderCatalogEntry } from "@cowboy/provider-ui";
import { ProviderIcon } from "./ProviderIcon";

interface Installation {
  value: string;
  label: string;
  provider: string;
  mode: "local" | "remote";
  entry: ProviderCatalogEntry;
}

export function AiInstallationPicker({
  installations,
  value,
  onChange,
  helperText,
}: {
  installations: readonly Installation[];
  value: string;
  onChange: (value: string) => void;
  helperText: string;
}): React.JSX.Element {
  const selected = installations.find((installation) =>
    installation.value === value
  );
  const icon = (installation: Installation) => (
    <ProviderIcon
      provider={installation.provider}
      providerVersion={installation.entry.provider_version}
      providerDigest={installation.entry.artifact_digest ?? undefined}
      fontSize="small"
      sx={{ flexShrink: 0 }}
    />
  );
  return (
    <TextField
      select
      label="AI installation"
      value={value}
      onChange={(event) => onChange(event.target.value)}
      helperText={helperText}
      slotProps={{
        select: {
          renderValue: () =>
            selected && (
              <Box
                sx={{
                  display: "flex",
                  alignItems: "center",
                  gap: 1.25,
                  minWidth: 0,
                }}
              >
                {icon(selected)}
                <Typography component="span" noWrap title={selected.label}>
                  {selected.label}
                </Typography>
              </Box>
            ),
        },
      }}
    >
      {installations.map((installation) => (
        <MenuItem
          key={installation.value}
          value={installation.value}
          sx={{
            alignItems: "center",
            minHeight: 44,
            py: 1,
            whiteSpace: "normal",
          }}
        >
          <ListItemIcon
            sx={{ width: 36, minWidth: 36, justifyContent: "center" }}
          >
            {icon(installation)}
          </ListItemIcon>
          <Box sx={{ minWidth: 0, overflowWrap: "anywhere" }}>
            <Typography>{installation.label}</Typography>
            <Typography variant="caption" color="text.secondary">
              {installation.mode === "remote" ? "Remote" : "Local"} ·{" "}
              {installation.entry.manifest.display.vendor}
            </Typography>
          </Box>
        </MenuItem>
      ))}
    </TextField>
  );
}
