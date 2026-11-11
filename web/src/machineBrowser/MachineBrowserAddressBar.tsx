/** Whose browser on which Machine: the identity line of the Desktop
 * modal's header and of the phone's bottom area (below its toolbar). The
 * remote Chromium shows its own address bar and tabs, so the panel adds
 * none. */
import { GroupsRounded } from "@mui/icons-material";
import { Box, Typography } from "@mui/material";

export function Identity({
  accountLabel,
  machineLabel,
  status,
  align,
  shared = false,
}: {
  accountLabel: string;
  machineLabel: string;
  status: string;
  align: "left" | "center";
  /** A shared browser: the group icon and accent mark it. */
  shared?: boolean;
}): React.JSX.Element {
  return (
    <Typography
      variant="caption"
      color="text.secondary"
      noWrap
      title={`${shared ? "Shared browser: " : ""}${accountLabel} · ${machineLabel} · ${status}`}
      data-machine-browser-identity={shared ? "shared" : "own"}
      sx={{ display: "block", minWidth: 0, textAlign: align, lineHeight: "16px" }}
    >
      {shared && (
        <GroupsRounded
          role="img"
          aria-label="Shared browser"
          sx={{ fontSize: "1.2em", mr: 0.5, verticalAlign: "-0.2em", color: "secondary.main" }}
        />
      )}
      <Box
        component="strong"
        sx={{ color: shared ? "secondary.main" : "text.primary", fontWeight: 600 }}
      >
        {accountLabel}
      </Box>
      {` · ${machineLabel} · ${status}`}
    </Typography>
  );
}
