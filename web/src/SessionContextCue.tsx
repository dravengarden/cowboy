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

/** Where a context cue renders. A pending choice changes the next send, so
 *  it stays on the Composer. Provenance is history: "Context from" opens the
 *  Transcript and "continued in" closes it, instead of a permanent card
 *  between the reply and the input. */
export type SessionContextCuePlacement = "composer" | "start" | "end";

function cuePlacement(kind: string): SessionContextCuePlacement {
  return kind === "continued_from"
    ? "start"
    : kind === "continued_in"
    ? "end"
    : "composer";
}

/** What the next message does (above the Composer), or where this
 *  conversation came from / continued (Transcript start / end). Ordinary
 *  sessions render nothing and fetch nothing. */
export function SessionContextCue(
  { sessionId, desktop = false, placement = "composer" }: {
    sessionId: string;
    desktop?: boolean;
    placement?: SessionContextCuePlacement;
  },
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
  if (!cue || !context || cuePlacement(cue.kind) !== placement) return null;
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
    ? "Continued in a new agent"
    : `Continued from ${cue.title}${cue.more > 0 ? ` +${cue.more}` : ""}`;
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
  const hibernate = (): void => {
    setBusy(true);
    setHibernateError("");
    void hibernateSession(sessionId)
      .catch((error: unknown) =>
        setHibernateError(
          error instanceof Error ? error.message : "Could not hibernate",
        )
      )
      .finally(() => setBusy(false));
  };
  if (placement !== "composer") {
    // A quiet divider in the conversation's own flow, like a date separator.
    return (
      <Box
        role="status"
        data-session-context-cue={cue.kind}
        data-transcript-context-marker={placement}
        sx={{
          display: "flex",
          alignItems: "center",
          gap: 1,
          px: desktop ? 2 : 1.5,
          py: placement === "start" ? 1.5 : 1,
          color: "text.secondary",
          "&::before, &::after": {
            content: '""',
            flex: "1 1 1.5rem",
            minWidth: "1.5rem",
            borderTop: 1,
            borderColor: "divider",
          },
        }}
      >
        <Stack
          direction="row"
          alignItems="center"
          spacing={0.5}
          sx={{ minWidth: 0, maxWidth: "min(100%, 34rem)" }}
        >
          <LinkOutlined
            sx={{ fontSize: glyphSize(16), flexShrink: 0, color: "primary.main" }}
          />
          <Typography
            variant="caption"
            noWrap
            sx={{ minWidth: 0 }}
            title={hibernateError || text}
          >
            {hibernateError || text}
          </Typography>
          <Button
            size="small"
            disabled={busy}
            onClick={action.run}
            sx={{ flexShrink: 0, minWidth: 0, px: 0.75, textTransform: "none" }}
          >
            {action.label}
          </Button>
          {canHibernate && (
            <Button
              size="small"
              disabled={busy}
              data-context-cue-hibernate
              title="Free this agent's memory; it resumes when you send here again"
              onClick={hibernate}
              sx={{ flexShrink: 0, minWidth: 0, px: 0.75, textTransform: "none" }}
            >
              Hibernate
            </Button>
          )}
        </Stack>
      </Box>
    );
  }
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
          title={text}
        >
          {text}
        </Typography>
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
