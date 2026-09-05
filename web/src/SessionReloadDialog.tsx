import Refresh from "@mui/icons-material/Refresh";
import {
  Button,
  Checkbox,
  CircularProgress,
  DialogContentText,
  FormControlLabel,
} from "@mui/material";
import { useEffect, useState } from "react";
import { Kbd, useConfirmEnter } from "./Kbd";
import { useNetworkActionState } from "./NetworkActionFeedback";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import type { SessionMeta } from "./protocol";
import {
  loadSessionReloadPlan,
  reloadSession,
  type SessionReloadPlan,
} from "./sessionReload";
import { ConfirmSheet } from "./Sheet";

export function SessionReloadDialog({
  session,
  onClose,
}: {
  session: SessionMeta | null | undefined;
  onClose: () => void;
}): React.JSX.Element {
  const action = useNetworkActionState();
  const open = session !== null && session !== undefined;
  const activeTurn = session?.status === "busy" ||
    session?.status === "starting";
  const planKey =
    `${session?.id}:${session?.status}:${session?.provider_generation_digest}`;
  const [loaded, setLoaded] = useState<
    { key: string; plan?: SessionReloadPlan; error?: string } | null
  >(null);
  const [keepPinned, setKeepPinned] = useState(false);
  const result = loaded?.key === planKey ? loaded : null;
  const plan = result?.plan;
  const upgrade = !activeTurn && plan?.upgrade_available === true &&
    !keepPinned;
  useEffect(() => {
    if (!session) return;
    let cancelled = false;
    setKeepPinned(false);
    void loadSessionReloadPlan(session.id).then(
      (plan) => {
        if (!cancelled) setLoaded({ key: planKey, plan });
      },
      (error: unknown) => {
        if (!cancelled) {
          setLoaded({
            key: planKey,
            error: error instanceof Error
              ? error.message
              : "Could not check Provider version",
          });
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [session?.id, planKey]);
  const confirm = (): void => {
    if (!session || (!activeTurn && !result)) return;
    void action.run(async () => {
      await reloadSession(session.id, {
        confirmActiveTurn: activeTurn,
        ...(upgrade && { providerGenerationDigest: plan.target_digest }),
      });
      onClose();
    });
  };
  useConfirmEnter(open, confirm);

  return (
    <ConfirmSheet
      open={open}
      onClose={(): void => {
        if (!action.pending) onClose();
      }}
      title="Reload this session?"
      actions={
        <>
          <Button
            color="inherit"
            onClick={onClose}
            disabled={action.pending}
            sx={{ minHeight: 44 }}
          >
            Cancel
            <Kbd keys="Esc" />
          </Button>
          <Button
            variant="contained"
            color={activeTurn ? "warning" : "primary"}
            startIcon={action.progress
              ? <CircularProgress size={16} color="inherit" />
              : <Refresh />}
            aria-busy={action.pending || undefined}
            disabled={action.pending || (!activeTurn && !result)}
            onClick={confirm}
            sx={{ minHeight: 44 }}
          >
            {activeTurn ? "Stop & reload" : "Reload"}
            <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
          </Button>
        </>
      }
    >
      <DialogContentText>
        Cowboy will rebuild the agent runtime and load the same session again.
        Conversation history, session ID, title, working directory, queue,
        drafts, and saved agent configuration are retained. If the new runtime
        no longer supports a saved setting, its supported value is used.
      </DialogContentText>
      {!result && !activeTurn && (
        <DialogContentText sx={{ mt: 1.5 }}>
          Checking installed Provider version…
        </DialogContentText>
      )}
      {plan?.upgrade_available && !activeTurn && (
        <FormControlLabel
          sx={{ mt: 1, alignItems: "center" }}
          control={
            <Checkbox
              checked={!keepPinned}
              disabled={action.pending}
              onChange={(_, checked): void => setKeepPinned(!checked)}
            />
          }
          label={`Load installed Provider ${plan.current_version} → ${plan.target_version}`}
        />
      )}
      {result && (
        <DialogContentText sx={{ mt: 1.5 }}>
          {upgrade
            ? "Only this idle session is upgraded. The same native conversation is resumed; failure will not create a blank session."
            : `Reload keeps Provider ${
              session?.provider_version || "current version"
            }.`}
          {result.error || plan?.blocked_reason
            ? ` ${result.error || plan?.blocked_reason}`
            : ""}
        </DialogContentText>
      )}
      {activeTurn && (
        <DialogContentText color="warning.main" sx={{ mt: 1.5 }}>
          The current turn will stop. Output already recorded in the transcript
          remains, but that turn may be incomplete.
        </DialogContentText>
      )}
    </ConfirmSheet>
  );
}
