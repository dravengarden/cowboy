import { Chip } from "@mui/material";
import { alpha } from "@mui/material/styles";
import type { AccountIdentity } from "./accountIdentity";
import { HintTooltip } from "./HintTooltip";

/** Passive metadata; the enclosing session/call keeps the touch target. */
export function AccountBadge({ account }: { account: AccountIdentity }): React.JSX.Element {
  return <HintTooltip title={account.description}>
    <Chip label={account.short} aria-label={account.description} size="small" data-account-badge={account.id ?? "unknown"}
      sx={{
        height: "1.25rem", maxWidth: "8rem", minWidth: 0, flexShrink: 0,
        fontSize: "0.6875rem", border: "1px solid transparent", boxShadow: "none",
        color: "text.secondary",
        bgcolor: (theme) => alpha(theme.palette.text.secondary, theme.palette.mode === "dark" ? 0.22 : 0.14),
        "& .MuiChip-label": { px: "0.5rem", overflow: "hidden", textOverflow: "ellipsis" },
      }} />
  </HintTooltip>;
}
