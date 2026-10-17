import Schedule from "@mui/icons-material/Schedule";
import Upgrade from "@mui/icons-material/Upgrade";
import { alpha, Box, Button, Stack, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import { useNetworkActionState } from "./NetworkActionFeedback";
import type { SessionMeta } from "./protocol";
import { providerName } from "./providerPresentation";
import {
  providerUpdateScheduleText,
  providerUpdateWaitsForTurn,
  requestProviderUpdateWhenIdle,
  sessionProviderUpdate,
} from "./providerUpdateOffer";

/** Touch-first notice in the session sheet: the one place a phone user looks
 *  for session state. Hidden entirely while no update waits. */
export function ProviderUpdateCard({
  session,
  onUpdate,
}: {
  session: SessionMeta;
  onUpdate: () => void;
}): React.JSX.Element | null {
  const action = useNetworkActionState();
  const offer = sessionProviderUpdate(session);
  const hasOffer = offer !== null;
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!hasOffer) return undefined;
    setNow(Date.now());
    const timer = globalThis.setInterval(() => setNow(Date.now()), 30_000);
    return () => globalThis.clearInterval(timer);
  }, [hasOffer]);
  if (!offer) return null;
  const scheduled = offer.when_idle === true;
  const waits = providerUpdateWaitsForTurn(session);
  const name = providerName(
    session.provider,
    session.provider_version,
    session.provider_generation_digest,
  );
  const schedule = providerUpdateScheduleText(offer, waits, now);
  const Icon = scheduled ? Schedule : Upgrade;

  return (
    <Box
      data-provider-update-card
      data-provider-update-scheduled={scheduled || undefined}
      role="status"
      sx={{
        // The Title field below floats its label above its border; keep a
        // clear gap so the card never crowds it.
        mt: 0.5,
        mb: 2,
        px: 1.5,
        py: 1.25,
        borderRadius: 2,
        border: 1,
        borderColor: (theme) => alpha(theme.palette.info.main, 0.35),
        bgcolor: (theme) => alpha(theme.palette.info.main, 0.08),
      }}
    >
      <Stack direction="row" spacing={1.25} alignItems="center">
        <Icon sx={{ color: "info.main", flexShrink: 0 }} />
        <Box sx={{ minWidth: 0, flex: 1 }}>
          <Typography variant="body2" sx={{ fontWeight: 650 }}>
            {scheduled
              ? `Update to ${offer.version} queued`
              : `Update to ${offer.version} available`}
          </Typography>
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ display: "block", fontVariantNumeric: "tabular-nums" }}
          >
            {name} {session.provider_version}
            {schedule ? ` · ${schedule}` : ""}
          </Typography>
        </Box>
        {scheduled
          ? (
            <Button
              size="small"
              color="inherit"
              disabled={action.pending}
              aria-label="cancel queued Provider update"
              onClick={(): void => {
                void action.run(() =>
                  requestProviderUpdateWhenIdle(session.id, false)
                );
              }}
              sx={{ minHeight: 44, flexShrink: 0, textTransform: "none" }}
            >
              Cancel
            </Button>
          )
          : (
            <Button
              size="small"
              variant="contained"
              color="info"
              disableElevation
              aria-label={`update ${name} to ${offer.version}`}
              onClick={onUpdate}
              sx={{
                minHeight: 40,
                px: 2,
                flexShrink: 0,
                textTransform: "none",
                fontWeight: 650,
              }}
            >
              Update
            </Button>
          )}
      </Stack>
    </Box>
  );
}
