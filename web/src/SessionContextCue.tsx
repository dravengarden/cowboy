import { useEffect, useRef, useState } from "react";
import { Alert, Box, Button, ButtonBase, keyframes, Stack, Typography } from "@mui/material";
import LinkOutlined from "@mui/icons-material/LinkOutlined";
import CallSplitOutlined from "@mui/icons-material/CallSplitOutlined";
import LinkOffOutlined from "@mui/icons-material/LinkOffOutlined";
import ExpandMore from "@mui/icons-material/ExpandMore";
import ChevronRightRounded from "@mui/icons-material/ChevronRightRounded";
import HistoryRounded from "@mui/icons-material/HistoryRounded";
import { glyphSize } from "./surface/desktopSize";
import { mobileComposerPanelFrameSx, mobileComposerPanelHeaderMinHeight } from "./mobileComposerPrimitives";
import { ProviderIcon } from "./ProviderIcon";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import {
  CONTEXT_CHANGED,
  contextConfigured,
  openContextDestination,
  saveSessionContext,
  withoutAgentSwitch,
  type SessionContext,
  useSessionContext,
} from "./sessionContext";
import { requestPickSession } from "./sessionPickRequest";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { accountIdentity, sessionAccountIdentity } from "./accountIdentity";
import { contextCue } from "./contextCue";
import { restoreItems, trashItems, useStoreSelector } from "./store";
import { SESSION_LINK_LABEL, sessionLinks, type SessionLinkRow } from "./sessionLinks";
import { requestSessionReveal } from "./sessionReveal";
import type { SessionMeta } from "./protocol";
import { hibernateAvailability, hibernateSession } from "./sessionHibernate";
import { duplicateMode, duplicateSession, forkAvailability, forkSession } from "./sessionFork";

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

/** The session that took over from this one, if any. Synced with the session
 *  list, so it holds after a reload and on every device; deleting the
 *  continuation returns its source to an ordinary session. */
export function useContinuation(sessionId: string): SessionMeta | undefined {
  return useStoreSelector((state) =>
    state.sessions.find((entry) => entry.continued_from === sessionId)
  );
}

/** A handed-off session is history: its Composer gives way to the dock unless
 *  its agent is still working, which must stay stoppable and observable. */
export function useHandedOff(sessionId: string): boolean {
  const continuation = useContinuation(sessionId);
  const status = useStoreSelector((state) =>
    state.sessions.find((entry) => entry.id === sessionId)?.status
  );
  return continuation !== undefined && status !== "busy" && status !== "starting";
}

/** Where a context cue renders. A pending choice changes the next send, so
 *  it stays on the Composer. Where the work continued stays in Session
 *  Settings with its actions. */
export type SessionContextCuePlacement = "composer" | "end";

const arrive = keyframes`
  from { opacity: 0; transform: translateY(6px); }
  to { opacity: 1; transform: none; }
`;

function cuePlacement(kind: string): SessionContextCuePlacement {
  return kind === "continued_in" ? "end" : "composer";
}

/** What the next message does (above the Composer), or where this
 *  conversation continued (Session Settings). Ordinary sessions render
 *  nothing and fetch nothing. */
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
    : "Continued in a new agent";
  const action = pending
    ? {
      label: busy ? "Cancelling…" : "Cancel",
      run: () => {
        setBusy(true);
        void saveSessionContext(sessionId, {
          revision: context.revision,
          ...(cue.kind === "pending_agent"
            ? withoutAgentSwitch(context, sessionId)
            : { sources: [], next_agent: null }),
        }).catch(() => undefined).finally(() => setBusy(false));
      },
    }
    : {
      label: "Open",
      run: () => {
        if (context.delivery) openContextDestination(context.delivery);
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

function openSessionLink(link: SessionLinkRow, session: SessionMeta | undefined): void {
  if (link.before === undefined) requestPickSession(link.id, session);
  else requestSessionReveal(link.id, session, link.before);
}

/** Announcements are per tab; after a reload the synced marks say to read. */
function useSessionLinks(sessionId: string): SessionLinkRow[] {
  const session = useStoreSelector((state) =>
    state.sessions.find((entry) => entry.id === sessionId)
  );
  const context = useAnnouncedSessionContext(sessionId, session?.continued_from !== undefined);
  const sessions = useStoreSelector((state) => state.sessions);
  const trashed = useStoreSelector((state) => state.trashedSessions);
  return sessionLinks(session, context?.links, sessions, trashed);
}

/** The first thing a continuation shows: one link back to where its work
 *  came from. Following it lands on the moment of the handoff. */
export function SessionContinuedFrom(
  { sessionId }: { sessionId: string },
): React.JSX.Element | null {
  const session = useStoreSelector((state) =>
    state.sessions.find((entry) => entry.id === sessionId)
  );
  const links = useSessionLinks(sessionId);
  const { data: accounts } = useSubscriptionAccounts();
  const source = links.find((link) => link.kind === "continued");
  const sourceSession = useStoreSelector((state) =>
    state.sessions.find((entry) => entry.id === source?.id)
  );
  if (!source || !session) return null;
  const openable = source.state === "open";
  const note = source.state === "trash" ? "in Trash" : source.state === "deleted" ? "deleted" : "";
  const more = links.length - 1;
  const title = `${source.title}${more > 0 ? ` +${more}` : ""}`;
  const here = sessionAccountIdentity(accounts, session).short;
  const was = sourceSession && sessionAccountIdentity(accounts, sourceSession).short;
  const change = was && was !== here ? `${was} → ${here}` : "";
  return (
    <Box sx={{ display: "flex", justifyContent: "center", flexShrink: 0, px: 1.5, pt: 1, maxWidth: "100%" }}>
      <ButtonBase
        data-session-handoff="start"
        disabled={!openable}
        aria-label={`Continued from ${title}${note && `, ${note}`}`}
        title={openable ? "Open where this conversation was handed off" : `This session is ${note}`}
        onClick={(): void => openSessionLink(source, sourceSession)}
        sx={{
          gap: 0.5,
          minWidth: 0,
          maxWidth: "100%",
          minHeight: 32,
          pl: 1.25,
          pr: 0.75,
          borderRadius: 999,
          color: "text.secondary",
          bgcolor: "action.hover",
          animation: `${arrive} 240ms ease-out`,
          "@media (prefers-reduced-motion: reduce)": { animation: "none" },
        }}
      >
        <HistoryRounded sx={{ fontSize: glyphSize(16), flexShrink: 0 }} />
        <Typography variant="caption" noWrap sx={{ minWidth: 0 }}>
          Continued from{" "}
          <Box component="span" sx={{ color: "text.primary", fontWeight: 600 }}>{title}</Box>
          {note ? ` · ${note}` : change && ` · ${change}`}
        </Typography>
        {openable && <ChevronRightRounded sx={{ fontSize: glyphSize(16), flexShrink: 0 }} />}
      </ButtonBase>
    </Box>
  );
}

/** Replaces the Composer of a handed-off session. Writing here would silently
 *  split one piece of work in two, so the dock offers what can still happen:
 *  go to the session that took over, deliberately branch this agent, or clear
 *  this history away. */
export function SessionHandedOffDock(
  { sessionId, desktop = false }: { sessionId: string; desktop?: boolean },
): React.JSX.Element | null {
  const session = useStoreSelector((state) =>
    state.sessions.find((entry) => entry.id === sessionId)
  );
  const machines = useStoreSelector((state) => state.machines);
  const continuation = useContinuation(sessionId);
  const { data: accounts } = useSubscriptionAccounts();
  const [busy, setBusy] = useState<"fork" | "trash" | null>(null);
  const [error, setError] = useState("");
  // A retry after an unknown outcome returns the same fork.
  const operation = useRef(`fork-${crypto.randomUUID()}`);
  if (!session || !continuation) return null;
  const openContinuation = (): void => requestPickSession(continuation.id, continuation);
  const run = (kind: "fork" | "trash", action: () => Promise<void>): void => {
    setBusy(kind);
    setError("");
    void action()
      .catch((failure: unknown) =>
        setError(failure instanceof Error ? failure.message : "That did not work")
      )
      .finally(() => setBusy(null));
  };
  const fork = async (): Promise<void> => {
    const account = session.provider_account_id || "default";
    const receipt = duplicateMode({
        sourceProvider: session.provider,
        sourceAccount: account,
        provider: session.provider,
        account,
        forkAvailable: forkAvailability(session, machines) === "ready",
      }) === "fork"
      ? await forkSession(sessionId, { operation: operation.current })
      : await duplicateSession(sessionId, {
        provider: session.provider,
        provider_account_id: account,
      }, operation.current);
    requestPickSession(receipt.session_id, receipt.session ?? undefined);
  };
  const account = sessionAccountIdentity(accounts, continuation).short;
  return (
    <Box
      data-session-handed-off
      data-composer-stack-slot="handed-off"
      sx={{
        display: "flex",
        flexDirection: "column",
        gap: 1,
        minWidth: 0,
        boxSizing: "border-box",
        p: desktop ? 1.5 : 1.25,
        ...(desktop
          ? { flexShrink: 0 }
          : { ...mobileComposerPanelFrameSx, bgcolor: "background.default" }),
      }}
    >
      <Box sx={{ minWidth: 0 }}>
        <Typography variant="body2" noWrap sx={{ fontWeight: 600 }}>
          Continued in {continuation.title || "a new agent"} · {account}
        </Typography>
        <Typography variant="caption" color="text.secondary" sx={{ display: "block" }}>
          This session is read-only history.
        </Typography>
      </Box>
      {error && <Alert severity="error">{error}</Alert>}
      <Stack direction="row" spacing={0.5} alignItems="center">
        <Button
          variant="contained"
          disableElevation
          disabled={busy !== null}
          onClick={openContinuation}
          sx={{ flex: 1, minWidth: 0, textTransform: "none" }}
        >
          Open
        </Button>
        <Button
          disabled={busy !== null}
          title="Start a separate session from this agent's conversation"
          onClick={() => run("fork", fork)}
          sx={{ flexShrink: 0, textTransform: "none" }}
        >
          {busy === "fork" ? "Forking…" : "Fork"}
        </Button>
        <Button
          color="inherit"
          disabled={busy !== null}
          title="The continuation can read this history until Trash retention ends"
          onClick={() =>
            run("trash", async () => {
              await trashItems({ sessions: [sessionId] });
              openContinuation();
            })}
          sx={{ flexShrink: 0, color: "text.secondary", textTransform: "none" }}
        >
          {busy === "trash" ? "Moving…" : "Move to Trash"}
        </Button>
      </Stack>
    </Box>
  );
}

/** One session this conversation came from or read. Open while it exists;
 *  a source in Trash can come back, a deleted one stays as a named record. */
function SessionLink({ link }: { link: SessionLinkRow }): React.JSX.Element {
  const session = useStoreSelector((state) =>
    state.sessions.find((entry) => entry.id === link.id)
  );
  const { data: accounts } = useSubscriptionAccounts();
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState("");
  const note = link.state === "trash" ? "In Trash" : link.state === "deleted" ? "Deleted" : "";
  const detail = [
    SESSION_LINK_LABEL[link.kind],
    session && sessionAccountIdentity(accounts, session).short,
    note,
  ].filter(Boolean).join(" · ");
  const open = link.state === "open";
  const Icon = link.state === "deleted"
    ? LinkOffOutlined
    : link.kind === "forked"
    ? CallSplitOutlined
    : LinkOutlined;
  return (
    <Stack direction="row" alignItems="center" spacing={0.5} sx={{ minWidth: 0 }}>
      <ButtonBase
        data-session-link={link.state}
        disabled={!open}
        title={open ? "Open at the point it was linked" : link.title}
        onClick={(): void => openSessionLink(link, session)}
        sx={{
          flex: 1,
          minWidth: 0,
          minHeight: 44,
          gap: 1,
          px: 0.5,
          borderRadius: 1,
          justifyContent: "flex-start",
          textAlign: "left",
        }}
      >
        <Icon
          sx={{
            fontSize: glyphSize(18),
            flexShrink: 0,
            color: open ? "primary.main" : "text.disabled",
          }}
        />
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography
            variant="body2"
            noWrap
            color={open ? "text.primary" : "text.secondary"}
          >
            {link.title}
          </Typography>
          <Typography
            variant="caption"
            noWrap
            color={failure ? "error" : "text.secondary"}
            sx={{ display: "block" }}
          >
            {failure || detail}
          </Typography>
        </Box>
        {open && (
          <ChevronRightRounded
            sx={{ fontSize: glyphSize(18), flexShrink: 0, color: "text.secondary" }}
          />
        )}
      </ButtonBase>
      {link.state === "trash" && (
        <Button
          size="small"
          disabled={busy}
          onClick={(): void => {
            setBusy(true);
            setFailure("");
            void restoreItems({ sessions: [link.id] })
              .catch((error: unknown) =>
                setFailure(error instanceof Error ? error.message : "Could not restore")
              )
              .finally(() => setBusy(false));
          }}
          sx={{ flexShrink: 0, minWidth: 0, textTransform: "none" }}
        >
          Restore
        </Button>
      )}
    </Stack>
  );
}

/** Every link of this session, and where it continued, for Session Settings.
 *  Renders nothing for an ordinary session. */
export function SessionProvenance(
  { sessionId }: { sessionId: string },
): React.JSX.Element | null {
  const links = useSessionLinks(sessionId);
  const context = useAnnouncedSessionContext(sessionId);
  const { data: accounts } = useSubscriptionAccounts();
  const cue = contextCue(
    sessionId,
    context,
    (provider, account) => accountIdentity(accounts, provider, account).label,
  );
  const continued = cue?.kind === "continued_in";
  const [expanded, setExpanded] = useState(false);
  if (links.length === 0 && !continued) return null;
  return (
    <Stack data-session-provenance-list spacing={0.5} sx={{ minWidth: 0, pb: 1.5 }}>
      {links.length > 0 && (
        <ButtonBase
          data-session-links-toggle
          aria-expanded={expanded}
          onClick={(): void => setExpanded((value) => !value)}
          sx={{
            alignSelf: "flex-start",
            gap: 0.25,
            minHeight: 32,
            borderRadius: 1,
            color: "text.secondary",
          }}
        >
          <Typography variant="caption">
            {links.length === 1 ? "1 link" : `${links.length} links`}
          </Typography>
          <ExpandMore
            sx={{
              fontSize: glyphSize(16),
              transform: expanded ? "rotate(180deg)" : undefined,
            }}
          />
        </ButtonBase>
      )}
      {expanded && links.map((link) => <SessionLink key={link.id} link={link} />)}
      <SessionContextCue sessionId={sessionId} placement="end" />
    </Stack>
  );
}
