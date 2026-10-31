import { useEffect, useState } from "react";
import { alpha, Box, Button, keyframes, Stack, Typography } from "@mui/material";
import LinkOutlined from "@mui/icons-material/LinkOutlined";
import CallSplitOutlined from "@mui/icons-material/CallSplitOutlined";
import EastRounded from "@mui/icons-material/EastRounded";
import { glyphSize } from "./surface/desktopSize";
import { mobileComposerPanelFrameSx, mobileComposerPanelHeaderMinHeight } from "./mobileComposerPrimitives";
import { ProviderIcon } from "./ProviderIcon";
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
import { accountIdentity, sessionAccountIdentity } from "./accountIdentity";
import { contextCue } from "./contextCue";
import { useStoreSelector } from "./store";
import { hibernateAvailability, hibernateSession } from "./sessionHibernate";

/** Context for one session, read only after the Service announced it. */
export function useAnnouncedSessionContext(
  sessionId: string,
  linked = false,
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
  return useSessionContext(sessionId, known || linked).data;
}

/** Where a context cue renders. A pending choice changes the next send, so
 *  it stays on the Composer. Provenance is history: Session Settings keeps its
 *  actions ("start": where it came from, "end": where it continued), and
 *  `SessionHandoffSeam` shows the same link in the conversation itself. */
export type SessionContextCuePlacement = "composer" | "start" | "end";

const arrive = keyframes`
  from { opacity: 0; transform: translateY(6px); }
  to { opacity: 1; transform: none; }
`;

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
    // One quiet provenance row in Session Settings.
    return (
      <Box
        role="status"
        data-session-context-cue={cue.kind}
        data-session-provenance={placement}
        sx={{ display: "flex", alignItems: "center", minWidth: 0, color: "text.secondary" }}
      >
        <Stack
          direction="row"
          alignItems="center"
          spacing={0.5}
          sx={{ minWidth: 0, maxWidth: "100%" }}
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
  const summary = cue.kind === "pending_agent"
    ? `Starts ${cue.label}`
    : cue.kind === "pending_links"
    ? `Links ${cue.count} session${cue.count === 1 ? "" : "s"} as context`
    : text;
  // One more row of the Prompt stack: it borrows the Plan panel's frame and
  // label grammar, and sits on the Composer whose next send it changes.
  return (
    <Box
      role="status"
      aria-live="polite"
      aria-label={text}
      data-session-context-cue={cue.kind}
      data-composer-stack-slot="context"
      sx={{
        display: "flex",
        alignItems: "center",
        gap: 1,
        minWidth: 0,
        boxSizing: "border-box",
        animation: `${arrive} 180ms ease-out`,
        "@media (prefers-reduced-motion: reduce)": { animation: "none" },
        ...(desktop
          ? {
            flexShrink: 0,
            px: 1,
            borderBottom: 1,
            borderColor: "divider",
          }
          : {
            ...mobileComposerPanelFrameSx,
            bgcolor: "background.default",
            pl: 1.25,
            pr: 0.5,
            minHeight: mobileComposerPanelHeaderMinHeight,
          }),
      }}
    >
      {cue.kind === "pending_agent"
        ? <ProviderIcon provider={cue.provider} sx={{ fontSize: glyphSize(18), flexShrink: 0 }} />
        : <LinkOutlined sx={{ fontSize: glyphSize(18), flexShrink: 0, color: "primary.main" }} />}
      <Typography
        variant={desktop ? "caption" : "overline"}
        sx={{ flexShrink: 0, lineHeight: 1.4, ...(desktop && { fontWeight: 600 }) }}
      >
        Next message
      </Typography>
      <Typography
        variant="caption"
        noWrap
        color="text.secondary"
        sx={{ flex: 1, minWidth: 0 }}
        title={text}
      >
        {summary}
      </Typography>
      <Button
        size="small"
        color="inherit"
        disabled={busy}
        onClick={action.run}
        sx={{ flexShrink: 0, minWidth: 0, color: "text.secondary", textTransform: "none" }}
      >
        {action.label}
      </Button>
    </Box>
  );
}

/** The two agents of a handoff, older behind newer. */
function HandoffMarks(
  { from, to }: { from?: string | undefined; to?: string | undefined },
): React.JSX.Element {
  const mark = (provider: string | undefined, behind: boolean): React.JSX.Element => (
    <Box
      sx={{
        display: "grid",
        placeItems: "center",
        width: 32,
        height: 32,
        flexShrink: 0,
        borderRadius: "50%",
        border: 1,
        borderColor: "divider",
        bgcolor: "background.paper",
        ...(behind ? { opacity: 0.72 } : { ml: -1.25 }),
      }}
    >
      {provider
        ? <ProviderIcon provider={provider} sx={{ fontSize: 18 }} />
        : <LinkOutlined sx={{ fontSize: 18, color: "primary.main" }} />}
    </Box>
  );
  return (
    <Box aria-hidden sx={{ display: "flex", flexShrink: 0 }}>
      {mark(from, true)}
      {mark(to, false)}
    </Box>
  );
}

/** Where this conversation came from (first row) or went (last row). The
 *  switch itself stays in Session Settings; this is the read-only record that
 *  makes the two sessions read as one piece of work. A short rail joins the
 *  card to the conversation it continues. */
export function SessionHandoffSeam(
  { sessionId, position, desktop = false }: {
    sessionId: string;
    position: "start" | "end";
    desktop?: boolean;
  },
): React.JSX.Element | null {
  const session = useStoreSelector((state) =>
    state.sessions.find((entry) => entry.id === sessionId)
  );
  // Announcements are per tab. After a reload only the synced continuation
  // mark says this session has a handoff to read, in either direction.
  const continued = useStoreSelector((state) =>
    state.sessions.some((entry) => entry.continued_from === sessionId)
  );
  const context = useAnnouncedSessionContext(
    sessionId,
    continued || session?.continued_from !== undefined,
  );
  const { data: accounts } = useSubscriptionAccounts();
  const delivery = context?.delivery;
  const stored = useStoreSelector((state) =>
    state.sessions.find((entry) => entry.id === delivery?.session_id)
  );
  const cue = contextCue(
    sessionId,
    context,
    (provider, account) => accountIdentity(accounts, provider, account).label,
  );
  if (!cue || cuePlacement(cue.kind) !== position) return null;
  const incoming = cue.kind === "continued_from";
  if (!incoming && cue.kind !== "continued_in") return null;
  const destination = stored ?? delivery?.session;
  const here = session && {
    provider: session.provider,
    label: sessionAccountIdentity(accounts, session).label,
  };
  const there = destination && {
    provider: destination.provider,
    label: sessionAccountIdentity(accounts, destination).label,
  };
  const from = incoming ? { provider: cue.provider, label: cue.label } : here;
  const to = incoming ? here : there;
  const title = incoming
    ? `${cue.title}${cue.more > 0 ? ` +${cue.more}` : ""}`
    : destination?.title || "A new agent";
  const agents = from?.label && to?.label
    ? from.label === to.label ? [to.label] : [from.label, to.label]
    : [];
  const rail = (
    <Box
      aria-hidden
      sx={(theme) => ({
        width: "2px",
        height: 16,
        bgcolor: alpha(theme.palette.primary.main, 0.4),
      })}
    />
  );
  return (
    <Box
      data-session-handoff={position}
      sx={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        flexShrink: 0,
        width: "100%",
        boxSizing: "border-box",
        px: desktop ? 2 : 1.5,
        pt: position === "start" ? 1.5 : 0,
        pb: position === "end" ? 1.5 : 0,
        animation: `${arrive} 240ms ease-out`,
        "@media (prefers-reduced-motion: reduce)": { animation: "none" },
      }}
    >
      {position === "end" && rail}
      <Stack
        role="group"
        aria-label={`${incoming ? "Continued from" : "Continued in"} ${title}`}
        direction="row"
        alignItems="center"
        spacing={1.25}
        sx={{
          width: "100%",
          maxWidth: 560,
          minWidth: 0,
          boxSizing: "border-box",
          pl: 1.25,
          pr: 0.75,
          py: 1,
          border: 1,
          borderColor: "divider",
          borderRadius: 1.5,
          bgcolor: "background.paper",
          textAlign: "left",
        }}
      >
        <HandoffMarks from={from?.provider} to={to?.provider} />
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography
            variant="overline"
            color="text.secondary"
            sx={{ display: "block", lineHeight: 1.4 }}
          >
            {incoming ? "Continued from" : "Continued in"}
          </Typography>
          <Typography variant="body2" noWrap title={title} sx={{ fontWeight: 600 }}>
            {title}
          </Typography>
          {agents.length > 0 && (
            <Stack
              direction="row"
              alignItems="center"
              flexWrap="wrap"
              useFlexGap
              sx={{ columnGap: 0.5, color: "text.secondary" }}
            >
              <Typography variant="caption">{agents[0]}</Typography>
              {agents[1] && (
                <>
                  <EastRounded
                    titleAccess="to"
                    sx={{ fontSize: glyphSize(14), color: "primary.main" }}
                  />
                  <Typography variant="caption">{agents[1]}</Typography>
                </>
              )}
            </Stack>
          )}
        </Box>
        <Button
          size="small"
          onClick={(): void => {
            if (incoming) requestPickSession(cue.id);
            else if (delivery) openContextDestination(delivery);
          }}
          sx={{ flexShrink: 0, minWidth: 0, textTransform: "none" }}
        >
          Open
        </Button>
      </Stack>
      {position === "start" && rail}
    </Box>
  );
}

/** Where this session came from and where it continued, for Session
 *  Settings. Renders nothing for an ordinary session. */
export function SessionProvenance(
  { sessionId }: { sessionId: string },
): React.JSX.Element | null {
  const forked = useStoreSelector((state) => {
    const session = state.sessions.find((entry) => entry.id === sessionId);
    const source = session?.forked_from;
    if (!source) return null;
    const title = state.sessions.find((entry) => entry.id === source)?.title;
    return { source, title: title || "another session" };
  });
  const context = useAnnouncedSessionContext(sessionId);
  const { data: accounts } = useSubscriptionAccounts();
  const cue = contextCue(
    sessionId,
    context,
    (provider, account) => accountIdentity(accounts, provider, account).label,
  );
  const continued = cue?.kind === "continued_from" || cue?.kind === "continued_in";
  if (!forked && !continued) return null;
  return (
    <Stack data-session-provenance-list spacing={0.25} sx={{ minWidth: 0 }}>
      {forked && (
        <Stack
          direction="row"
          alignItems="center"
          spacing={0.5}
          data-session-provenance="forked"
          sx={{ minWidth: 0, color: "text.secondary" }}
        >
          <CallSplitOutlined
            sx={{ fontSize: glyphSize(16), flexShrink: 0, color: "primary.main" }}
          />
          <Typography variant="caption" noWrap sx={{ minWidth: 0 }} title={forked.title}>
            Forked from {forked.title}
          </Typography>
          <Button
            size="small"
            onClick={() => requestPickSession(forked.source)}
            sx={{ flexShrink: 0, minWidth: 0, px: 0.75, textTransform: "none" }}
          >
            Open
          </Button>
        </Stack>
      )}
      <SessionContextCue sessionId={sessionId} placement="start" />
      <SessionContextCue sessionId={sessionId} placement="end" />
    </Stack>
  );
}
