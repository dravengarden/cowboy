import Chip from "@mui/material/Chip";
import Schedule from "@mui/icons-material/Schedule";
import Upgrade from "@mui/icons-material/Upgrade";
import { alpha } from "@mui/material/styles";
import type { SessionMeta } from "./protocol";
import { sessionProviderUpdate } from "./providerUpdateOffer";
import { HintTooltip } from "./HintTooltip";

/** Session-list notice that a newer Provider release waits for this session.
 *  Passive: a tap opens the session, whose sheet or top bar offers the update. */
export function SessionUpdateBadge({ session }: { session: SessionMeta }) {
  const offer = sessionProviderUpdate(session);
  const waiting = session.resource_wait;
  if (!offer && !waiting) return null;
  const queued = Boolean(waiting) || offer?.when_idle === true;
  const description = waiting ? `Waiting for resources: ${waiting}` : queued
    ? `Updates to ${offer?.version ?? ""} after the current turn`
    : `Provider update available: ${session.provider_version ?? ""} → ${offer?.version ?? ""}`;
  return (
    <HintTooltip title={description}>
      <Chip
        data-session-update-badge
        data-session-update-queued={queued || undefined}
        size="small"
        variant="filled"
        color="info"
        label={waiting ? (offer ? `Waiting · ${offer.version}` : "Waiting") : offer?.version}
        aria-label={description}
        icon={queued ? <Schedule /> : <Upgrade />}
        sx={{
          height: "1.25rem",
          flexShrink: 0,
          fontSize: "0.6875rem",
          fontVariantNumeric: "tabular-nums",
          border: "1px solid transparent",
          color: "info.main",
          bgcolor: (theme) => alpha(theme.palette.info.main, theme.palette.mode === "dark" ? 0.22 : 0.14),
          "& .MuiChip-icon": { fontSize: "0.8125rem", ml: "0.3rem", mr: "-0.2rem", color: "inherit" },
          // A version is short and meaningless when cut; never ellipsize it.
          maxWidth: "none",
          "& .MuiChip-label": { px: "0.45rem", overflow: "visible" },
        }}
      />
    </HintTooltip>
  );
}
