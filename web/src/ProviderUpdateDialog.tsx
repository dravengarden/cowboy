import { desktopSize } from "./surface/desktopSize";
import Schedule from "@mui/icons-material/Schedule";
import Upgrade from "@mui/icons-material/Upgrade";
import {
  Box,
  Button,
  CircularProgress,
  DialogContentText,
  Stack,
  Typography,
} from "@mui/material";
import { useEffect, useState } from "react";
import { Kbd, useConfirmEnter } from "./Kbd";
import { useNetworkActionState } from "./NetworkActionFeedback";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import type { SessionMeta } from "./protocol";
import { providerName } from "./providerPresentation";
import {
  providerUpdateRepinsOnly,
  providerUpdateScheduleText,
  providerUpdateWaitsForTurn,
  requestProviderUpdateWhenIdle,
  sessionProviderUpdate,
} from "./providerUpdateOffer";
import { reloadSession } from "./sessionReload";
import { ConfirmSheet } from "./Sheet";

/** Confirm adopting the newer installed Provider release. Shared by the
 *  Desktop top bar and the Mobile session sheet; the sheet surface itself
 *  adapts (centered dialog on Desktop, inset card on touch). */
export function ProviderUpdateDialog({
  session,
  onClose,
}: {
  session: SessionMeta | null | undefined;
  onClose: () => void;
}): React.JSX.Element {
  const action = useNetworkActionState();
  const offer = sessionProviderUpdate(session);
  const open = session !== null && session !== undefined && offer !== null;
  const waits = providerUpdateWaitsForTurn(session);
  const repinOnly = providerUpdateRepinsOnly(session);
  const scheduled = offer?.when_idle === true;
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!open) return undefined;
    setNow(Date.now());
    const timer = globalThis.setInterval(() => setNow(Date.now()), 30_000);
    return () => globalThis.clearInterval(timer);
  }, [open]);
  // The offer disappears once the update starts (or another device applied
  // it); the dialog has nothing left to ask.
  useEffect(() => {
    if (session && offer === null && !action.pending) onClose();
  }, [session, offer, action.pending, onClose]);

  const name = session
    ? providerName(
      session.provider,
      session.provider_version,
      session.provider_generation_digest,
    )
    : "";
  const confirm = (): void => {
    if (!session || !offer) return;
    void action.run(async () => {
      if (scheduled) {
        await requestProviderUpdateWhenIdle(session.id, false);
      } else if (waits || repinOnly) {
        await requestProviderUpdateWhenIdle(session.id, true);
      } else {
        await reloadSession(session.id, {
          providerGenerationDigest: offer.digest,
        });
      }
      onClose();
    });
  };
  useConfirmEnter(open && !scheduled, confirm);
  const schedule = offer && !scheduled
    ? providerUpdateScheduleText(offer, waits, now)
    : null;
  const primary = scheduled
    ? "Cancel update"
    : waits
    ? "Update after this turn"
    : repinOnly
    ? "Update on next open"
    : "Update now";

  return (
    <ConfirmSheet
      open={open}
      onClose={(): void => {
        if (!action.pending) onClose();
      }}
      title={`Update ${name}`}
      actions={
        <>
          <Button
            color="inherit"
            onClick={onClose}
            disabled={action.pending}
            sx={{ minHeight: 44 }}
          >
            {scheduled ? "Keep" : "Later"}
            <Kbd keys="Esc" />
          </Button>
          <Button
            data-provider-update-confirm
            variant={scheduled ? "outlined" : "contained"}
            color={scheduled ? "inherit" : "info"}
            startIcon={action.progress
              ? <CircularProgress size={desktopSize(16)} color="inherit" />
              : waits && !scheduled
              ? <Schedule />
              : scheduled
              ? undefined
              : <Upgrade />}
            aria-busy={action.pending || undefined}
            disabled={action.pending}
            onClick={confirm}
            sx={{ minHeight: 44 }}
          >
            {primary}
            {!scheduled && <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />}
          </Button>
        </>
      }
    >
      <Stack spacing={1.5}>
        <Stack
          direction="row"
          spacing={1}
          alignItems="center"
          data-provider-update-versions
          sx={{ fontVariantNumeric: "tabular-nums" }}
        >
          <Typography variant="body2" color="text.secondary">
            {session?.provider_version || "current"}
          </Typography>
          <Typography variant="body2" color="text.disabled" aria-hidden>
            →
          </Typography>
          <Typography variant="body2" fontWeight={750} color="info.main">
            {offer?.version}
          </Typography>
        </Stack>
        <DialogContentText>
          {scheduled
            ? waits
              ? "This session updates as soon as the current turn finishes. Nothing is interrupted."
              : "This session is updating shortly."
            : repinOnly
            ? "The session is not running. It opens on the new release next time, with the same conversation; nothing starts now."
            : "Cowboy restarts the agent runtime on the new release and resumes this same conversation. History, queue, drafts, and settings are kept."}
        </DialogContentText>
        {!scheduled && waits && (
          <DialogContentText>
            The current turn keeps running; the update starts once the session
            is idle.
          </DialogContentText>
        )}
        {(schedule || !scheduled) && (
          <Box>
            {schedule && (
              <Typography
                variant="caption"
                color="text.secondary"
                display="block"
              >
                {`If you do nothing: ${schedule}.`}
              </Typography>
            )}
            {!scheduled && !repinOnly && (
              <Typography
                variant="caption"
                color="text.secondary"
                display="block"
              >
                The next prompt may not reuse the provider's prompt cache.
              </Typography>
            )}
          </Box>
        )}
      </Stack>
    </ConfirmSheet>
  );
}
