import { Box, Stack, Typography } from "@mui/material";
import type { ReactNode } from "react";
import type { ProviderUiManifest } from "@cowboy/provider-ui";
import { ProviderMarkStack } from "./ProviderSurface";

export const providerAccountCardSx = {
  p: 1.5, minWidth: 0, borderRadius: 1.25, borderColor: "divider",
  bgcolor: "transparent", boxShadow: "none",
};

/** Service credentials and Machine subscriptions share one resting card.
 * Credential ownership affects the facts and actions, never the visual hierarchy. */
export function ProviderAccountIdentity({ manifests, title, summary, actions, children }: {
  manifests: readonly ProviderUiManifest[];
  title: string;
  summary: string;
  actions: ReactNode;
  children: ReactNode;
}): React.JSX.Element {
  const shared = manifests.length > 1;
  return <Stack data-provider-management-identity data-provider-shared-identity={shared ? "true" : undefined}
    spacing={0.75} sx={{ minWidth: 0 }}>
    <Stack direction="row" alignItems="center" spacing={1} sx={{ minWidth: 0 }}>
      {manifests.length === 1 && <Box data-provider-management-mark sx={{
        display: "flex", alignItems: "center", justifyContent: "center", minWidth: 32, minHeight: 32, flexShrink: 0, color: "text.primary",
      }}>
        <ProviderMarkStack manifests={manifests} size={26} monochromeColor="inherit" />
      </Box>}
      <Typography variant="body2" fontWeight={650} sx={{ flex: 1, minWidth: 0, lineHeight: 1.3, overflowWrap: "anywhere" }}>{title}</Typography>
      <Box data-provider-management-actions sx={{
        ml: "auto", flex: "0 0 auto", display: "flex", justifyContent: "flex-end", minWidth: 0,
        "& > .MuiStack-root": { alignItems: "center", justifyContent: "flex-end", flexDirection: "row", flexWrap: "wrap", gap: 0.75, width: "auto" },
        "& .MuiButton-root": { minHeight: 32, px: 1.25, flex: "0 0 auto", fontWeight: 650, letterSpacing: 0, textTransform: "none", boxShadow: "none" },
        "& .MuiButton-contained": { bgcolor: "transparent", color: "primary.main", border: "1px solid", borderColor: "divider", "&:hover": { bgcolor: "action.hover" } },
      }}>{actions}</Box>
    </Stack>
    {shared && <Box data-provider-management-mark data-provider-credential-consumers sx={{ minWidth: 0 }}>
      <ProviderMarkStack manifests={manifests} labeled size={20} />
    </Box>}
    <Typography variant="caption" color="text.secondary" sx={{ lineHeight: 1.35, overflowWrap: "anywhere" }}>{summary}</Typography>
    {children}
  </Stack>;
}

export function ProviderAccountStatus({ label, tone = "default", detail }: {
  label: string; tone?: "default" | "success" | "warning"; detail?: string;
}): React.JSX.Element {
  return <Stack data-provider-management-footer direction="row" alignItems="center" flexWrap="wrap" sx={{ minWidth: 0 }}>
    <Typography data-provider-management-status variant="caption" color={tone === "success" ? "success.main" : tone === "warning" ? "warning.main" : "text.secondary"}
      sx={{ overflowWrap: "anywhere" }}>{label.charAt(0).toUpperCase() + label.slice(1)}</Typography>
    {detail && <Typography variant="caption" color="text.secondary" sx={{ overflowWrap: "anywhere", whiteSpace: "pre-wrap" }}>{` · ${detail}`}</Typography>}
  </Stack>;
}
