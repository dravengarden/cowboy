import { useEffect, useState } from "react";
import { alpha, Box, Button, Stack, Typography } from "@mui/material";
import LinkOutlined from "@mui/icons-material/LinkOutlined";
import { glyphSize } from "./surface/desktopSize";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import {
  CONTEXT_CHANGED,
  contextConfigured,
  openContextDestination,
  saveSessionContext,
  type SessionContext,
  useSessionContext,
} from "./sessionContext";
import { requestPickSession } from "./sessionPickRequest";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { accountIdentity } from "./accountIdentity";
import { contextCue } from "./contextCue";
import { useStoreSelector } from "./store";
import { hibernateAvailability, hibernateSession } from "./sessionHibernate";

/** Context for one session, read only after the Service announced it. */
export function useAnnouncedSessionContext(
  sessionId: string,
): SessionContext | undefined {
  const [known, setKnown] = useState(() => contextConfigured(sessionId));
  useEffect(() => {
    setKnown(contextConfigured(sessionId));
    const check = () => setKnown(contextConfigured(sessionId));
    window.addEventListener(CONTEXT_CHANGED, check);
    window.addEventListener(PREFERENCES_CHANGED, check);
    return () => {
      window.removeEventListener(CONTEXT_CHANGED, check);
      window.removeEventListener(PREFERENCES_CHANGED, check);
    };
  }, [sessionId]);
  return useSessionContext(sessionId, known).data;
}

/** One line above the Composer: what the next message does, or where this
 *  conversation continued. Ordinary sessions render nothing and fetch nothing. */
export function SessionContextCue(
  { sessionId, desktop = false }: { sessionId: string; desktop?: boolean },
): React.JSX.Element | null {
  const context = useAnnouncedSessionContext(sessionId);
  const { data: accounts } = useSubscriptionAccounts();
  const [busy, setBusy] = useState(false);
  const [hibernateError, setHibernateError] = useState("");
  const session = useStoreSelector((state) =>
    state.sessions.find((entry) => entry.id === sessionId)
  );
  const machines = useStoreSelector((state) => state.machines);
  const cue = contextCue(
    sessionId,
    context,
    (provider, account) => accountIdentity(accounts, provider, account).label,
  );
  if (!cue || !context) return null;
  // The original agent usually goes idle after a handoff; offer to free its
  // worker without stopping anything automatically.
  const canHibernate = cue.kind === "continued_in" && session !== undefined &&
    hibernateAvailability(session, machines) === "ready";
  const pending = cue.kind === "pending_agent" || cue.kind === "pending_links";
  const text = cue.kind === "pending_agent"
    ? `Your next message starts a new agent · ${cue.label}`
    : cue.kind === "pending_links"
    ? `Your next message links ${cue.count} session${
      cue.count === 1 ? "" : "s"
    } as context`
    : cue.kind === "continued_in"
    ? "This conversation continued in a new agent"
    : `Context from ${cue.title}${cue.more > 0 ? ` +${cue.more}` : ""}`;
  const action = pending
    ? {
      label: busy ? "Cancelling…" : "Cancel",
      run: () => {
        setBusy(true);
        void saveSessionContext(sessionId, {
          revision: context.revision,
          sources: [],
          next_agent: null,
        }).catch(() => undefined).finally(() => setBusy(false));
      },
    }
    : cue.kind === "continued_in"
    ? {
      label: "Open",
      run: () => {
        if (context.delivery) openContextDestination(context.delivery);
      },
    }
    : {
      label: "Open",
      run: () => {
        if (cue.kind === "continued_from") requestPickSession(cue.id);
      },
    };
  return (
    <Box
      data-session-context-cue={cue.kind}
      data-composer-stack-slot="context"
      sx={{
        display: "flex",
        justifyContent: "center",
        px: desktop ? 0 : 2,
        ...(desktop && { flexShrink: 0 }),
        width: "100%",
        minWidth: 0,
        boxSizing: "border-box",
      }}
    >
      <Stack
        role="status"
        aria-live="polite"
        direction="row"
        alignItems="center"
        spacing={1}
        sx={(theme) => ({
          width: "100%",
          maxWidth: desktop ? "none" : 560,
          minWidth: 0,
          px: 1.25,
          py: 0.25,
          border: desktop ? 0 : 1,
          ...(desktop && { borderBottom: 1 }),
          borderColor: alpha(theme.palette.primary.main, pending ? 0.4 : 0.2),
          borderRadius: desktop ? 0 : 2,
          bgcolor: alpha(
            theme.palette.primary.main,
            pending ? (theme.palette.mode === "dark" ? 0.14 : 0.07) : 0.03,
          ),
        })}
      >
        <LinkOutlined
          sx={{
            fontSize: glyphSize(18),
            flexShrink: 0,
            color: "primary.main",
          }}
        />
        <Typography
          variant="caption"
          sx={{
            flex: 1,
            minWidth: 0,
            lineHeight: 1.35,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
          title={hibernateError || text}
        >
          {hibernateError || text}
        </Typography>
        {canHibernate && (
          <Button
            size="small"
            disabled={busy}
            data-context-cue-hibernate
            title="Free this agent's memory; it resumes when you send here again"
            onClick={() => {
              setBusy(true);
              setHibernateError("");
              void hibernateSession(sessionId)
                .catch((error: unknown) =>
                  setHibernateError(
                    error instanceof Error
                      ? error.message
                      : "Could not hibernate",
                  )
                )
                .finally(() => setBusy(false));
            }}
            sx={{ flexShrink: 0, minWidth: 0, textTransform: "none" }}
          >
            Hibernate
          </Button>
        )}
        <Button
          size="small"
          disabled={busy}
          onClick={action.run}
          sx={{ flexShrink: 0, minWidth: 0, textTransform: "none" }}
        >
          {action.label}
        </Button>
      </Stack>
    </Box>
  );
}
