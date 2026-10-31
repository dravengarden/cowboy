import { useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  Box,
  Button,
  Checkbox,
  FormControlLabel,
  Stack,
  Typography,
} from "@mui/material";
import { ExpandMore, ForkRightOutlined } from "@mui/icons-material";
import { isManagedChild } from "./managedCalls";
import type { SessionMeta } from "./protocol";
import { useStoreSelector } from "./store";
import { sessionExecution } from "./sessionExecution";
import { SessionContextPicker } from "./SessionContextPicker";
import {
  openContextDestination,
  saveSessionContext,
  type SessionContext,
  pendingSources,
  trackLinkEdit,
  useSessionContext,
  withoutAgentSwitch,
} from "./sessionContext";
import { useProjectPlacement } from "./useProjectPlacement";
import { WorkspacePicker } from "./WorkspacePicker";
import { AiInstallationPicker } from "./AiInstallationPicker";
import { SessionPresetPicker } from "./SessionPresetPicker";
import {
  SessionSubscription,
  useSessionSubscription,
} from "./SessionSubscription";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { ConfirmSheet } from "./Sheet";
import { SessionProvenance, useContinuation } from "./SessionContextCue";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { accountIdentity } from "./accountIdentity";
import {
  duplicateMode,
  duplicateSession,
  forkAvailability,
  forkSession,
} from "./sessionFork";
import { requestPickSession } from "./sessionPickRequest";
import {
  closeSessionSettings,
  requestComposerFocus,
  SESSION_NEW_FROM_EVENT,
  takeNewFromRequest,
} from "./sessionSettingsOpen";
import { newUuid } from "./uuid";

/** Three questions, one place each. How this agent runs lives in the Agent
 *  block above. Here: what the next message can read (Link sessions), and
 *  where the work goes (Continue or fork). */
export function SessionContextSettings(
  { session, provenance = true, allowFork = true }: {
    session: SessionMeta;
    provenance?: boolean;
    /** An empty session has no conversation to branch. */
    allowFork?: boolean;
  },
): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<string[] | null>(null);
  const section = useRef<HTMLDivElement>(null);
  const { data, error, reload } = useSessionContext(session.id);
  const handedOff = useContinuation(session.id) !== undefined;
  useEffect(() => {
    const take = () => {
      if (takeNewFromRequest(session.id)) setOpen(true);
    };
    take();
    document.addEventListener(SESSION_NEW_FROM_EVENT, take);
    return () => document.removeEventListener(SESSION_NEW_FROM_EVENT, take);
  }, [session.id]);
  useEffect(() => {
    if (open) section.current?.scrollIntoView({ block: "nearest" });
  }, [open]);
  const onSaved = () => {
    void reload();
    setOpen(false);
  };
  const stagedAgent = Boolean(data?.next_agent) &&
    data?.revision !== data?.applied_revision;
  return (
    <Box
      ref={section}
      data-session-context
      sx={{ minWidth: 0, pb: open ? 2 : 0 }}
    >
      {provenance && <SessionProvenance sessionId={session.id} />}
      {/* Its work moved on: a further switch from here would split it. */}
      {!handedOff && (
        <Stack spacing={1.5}>
          {error && (
            <Alert
              severity="error"
              action={<Button onClick={() => void reload()}>Retry</Button>}
            >
              {error}
            </Alert>
          )}
          {data && (
            <>
              <ContextDelivery
                session={session}
                context={data}
                onResolved={onSaved}
              />
              <LinkedSessions
                session={session}
                context={data}
                draft={draft}
                onDraft={setDraft}
                onSettled={reload}
              />
            </>
          )}
          <Box>
            <Button
              startIcon={<ForkRightOutlined />}
              endIcon={
                <ExpandMore sx={{ transform: open ? "rotate(180deg)" : undefined }} />
              }
              aria-expanded={open}
              aria-keyshortcuts="Enter"
              disabled={!data}
              // Functional: a save that just closed the form may not have
              // re-rendered yet, and a stale `open` would close it again.
              onClick={() => setOpen((value) => !value)}
              sx={{ justifyContent: "flex-start", textTransform: "none" }}
            >
              Continue or fork
            </Button>
            {data && stagedAgent && !open && (
              <PendingSwitch session={session} context={data} onBack={() => void reload()} />
            )}
          </Box>
          {data && open && (
            <NewSessionForm
              key={session.id}
              session={session}
              observed={data}
              sources={draft ?? pendingSources(data)}
              allowFork={allowFork && !session.system && !isManagedChild(session)}
              onContinued={() => {
                onSaved();
                // A staged choice applies to the next message: go write it.
                closeSessionSettings(session.id);
                requestComposerFocus(session.id);
              }}
              onForked={() => setOpen(false)}
              onCancel={() => setOpen(false)}
            />
          )}
        </Stack>
      )}
    </Box>
  );
}

/** What the next prompt will read, whichever agent receives it. Like an
 *  attachment: each change applies at once and nothing runs until that prompt
 *  is sent, so there is nothing to save. A fork leaves this selection here. */
function LinkedSessions(
  { session, context, draft, onDraft, onSettled }: {
    session: SessionMeta;
    context: SessionContext;
    draft: string[] | null;
    onDraft: (draft: string[] | null) => void;
    onSettled: () => Promise<unknown>;
  },
): React.JSX.Element {
  const [error, setError] = useState("");
  const [recovering, setRecovering] = useState(false);
  const latest = useRef<string[] | null>(null);
  const applying = useRef(false);
  const revision = useRef(context.revision);
  if (!applying.current) revision.current = context.revision;
  const unresolved = context.delivery?.state === "preparing" ||
    context.delivery?.state === "unknown";
  // Selections made while one saves collapse into the newest.
  const apply = async () => {
    if (applying.current) return;
    applying.current = true;
    setError("");
    try {
      while (latest.current) {
        const sources = latest.current;
        latest.current = null;
        const saved = await saveSessionContext(session.id, {
          revision: revision.current,
          sources,
          next_agent: context.next_agent,
        });
        revision.current = saved.revision;
        // Keep the selection on screen until the saved one replaces it.
        if (!latest.current) await onSettled();
      }
    } catch (cause) {
      latest.current = null;
      setRecovering(true);
      setError(cause instanceof Error ? cause.message : "Could not link sessions");
      await onSettled();
      setRecovering(false);
    } finally {
      applying.current = false;
      onDraft(null);
    }
  };
  return (
    <Stack spacing={1} data-context-links>
      <Typography variant="caption" color="text.secondary">
        Link sessions · Read with your next prompt, no reply until then
      </Typography>
      <SessionContextPicker
        value={draft ?? pendingSources(context)}
        onChange={(next) => {
          onDraft(next);
          latest.current = next;
          trackLinkEdit(session.id, apply());
        }}
        disabled={unresolved || recovering}
        // Continuing makes this session a source of the new agent's context.
        exclude={context.next_agent ? undefined : session.id}
      />
      {error && <Alert severity="error">{error}</Alert>}
    </Stack>
  );
}

/** Where the work goes: one target, then either move the work there or keep
 *  both lines live. How history travels follows from the target. */
function NewSessionForm(
  { session, observed, sources, allowFork, onContinued, onForked, onCancel }: {
    session: SessionMeta;
    observed: SessionContext;
    sources: string[];
    allowFork: boolean;
    onContinued: () => void;
    onForked: () => void;
    onCancel: () => void;
  },
): React.JSX.Element {
  const [snapshot] = useState(observed);
  // Links are edited beside this form; only what it replaces is a conflict.
  const changedElsewhere =
    JSON.stringify(observed.next_agent) !== JSON.stringify(snapshot.next_agent) ||
    observed.delivery?.operation !== snapshot.delivery?.operation;
  const [advanced, setAdvanced] = useState(false);
  const [inheritSettings, setInheritSettings] = useState(
    snapshot.next_agent?.inherit_settings ?? true,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [preset, setPreset] = useState(snapshot.next_agent?.preset || "");
  const machines = useStoreSelector((state) => state.machines);
  const execution = sessionExecution(session);
  const defaults = {
    project: snapshot.next_agent?.cwd || session.workspace_id || null,
    machine: snapshot.next_agent?.machine_id || execution.machineId || null,
    runtime_machine: snapshot.next_agent?.runtime_machine_id ||
      session.machine_id || null,
    provider: snapshot.next_agent?.provider || session.provider,
  };
  const placement = useProjectPlacement(true, machines, defaults);
  const provider = placement.installation?.provider || defaults.provider;
  const subscription = useSessionSubscription(true, provider);
  const desktop = useSurfaceProfile().kind === "desktop";
  const unresolved = observed.delivery?.state === "preparing" ||
    observed.delivery?.state === "unknown";
  const presets =
    placement.installation?.entry.manifest.configuration.presets ?? [];
  const validPreset = !preset ||
    presets.some((entry) => `${provider}/${entry.id}` === preset);
  const sourceAccount = session.provider_account_id || "default";
  const account = snapshot.next_agent?.provider_account_id || sourceAccount;
  useEffect(() => {
    if (provider === defaults.provider) subscription.select(account);
  }, [provider, account]);
  const nextAgent = {
    provider,
    provider_account_id: subscription.value,
    machine_id: placement.machineId,
    runtime_machine_id: placement.runtimeMachineId || null,
    cwd: placement.project?.projectId || "",
    preset: preset || null,
    inherit_settings: inheritSettings,
  };
  const sameAccount = provider === session.provider &&
    subscription.value === sourceAccount;
  // Worktrees live on the execution Machine, per project.
  const sameHome = nextAgent.cwd === (session.workspace_id || "") &&
    nextAgent.machine_id === (execution.machineId || null);
  const copiesWorktree = sameHome && machines.some((machine) =>
    machine.id === nextAgent.machine_id && machine.capabilities?.native_fork
  );
  // A fork stays in this session's project and Machines.
  const samePlacement = sameHome &&
    (nextAgent.runtime_machine_id || session.machine_id) === session.machine_id;
  const fork = forkAvailability(session, machines);
  // A native fork takes the conversation as it is, so a chosen preset or
  // cleared inheritance needs a new agent rather than being ignored.
  const forkMode = duplicateMode({
    sourceProvider: session.provider,
    sourceAccount,
    provider,
    account: subscription.value,
    forkAvailable: fork !== null && !preset && inheritSettings,
  });
  const waitForTurn = forkMode === "fork" && fork === "busy";
  // One operation per distinct request: a retry returns the same copy, and
  // changing any choice asks for a new one.
  const choice = JSON.stringify([provider, subscription.value, preset, inheritSettings]);
  const key = useMemo(() => `duplicate-${newUuid()}`, [choice]);
  // The first send needs this session as a source unless it was removed from
  // an already staged continuation.
  const continueSources = snapshot.next_agent || sources.includes(session.id)
    ? sources
    : [session.id, ...sources];
  const full = continueSources.length > observed.max_sources;
  const ready = placement.ready && subscription.allowed && validPreset;
  const locked = busy || changedElsewhere || unresolved;
  const run = async (action: () => Promise<void>, failure: string) => {
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : failure);
    } finally {
      setBusy(false);
    }
  };
  const continueWork = () =>
    run(async () => {
      await saveSessionContext(session.id, {
        revision: observed.revision,
        sources: continueSources,
        next_agent: nextAgent,
      });
      onContinued();
    }, "Could not save context");
  const forkWork = () =>
    run(async () => {
      const receipt = forkMode === "fork"
        ? await forkSession(session.id, { operation: key })
        : await duplicateSession(session.id, {
          provider,
          provider_account_id: subscription.value,
          preset: preset || null,
          inherit_settings: inheritSettings,
        }, key);
      onForked();
      closeSessionSettings(session.id);
      requestPickSession(receipt.session_id, receipt.session ?? undefined);
    }, "Fork failed");
  return (
    <Stack spacing={1.5} data-new-session-editor>
      {changedElsewhere && (
        <Alert severity="warning">
          Context changed on another device. Close and reopen this section to
          review it; your choices have not been applied.
        </Alert>
      )}
      <AiInstallationPicker
        installations={placement.installations}
        value={placement.installation?.value || ""}
        onChange={(value) => {
          if (
            placement.installations.find((item) => item.value === value)
              ?.provider !== provider
          ) setPreset("");
          placement.selectInstallation(value);
        }}
        helperText={placement.error ||
          (placement.loading ? "Checking available installations…" : "")}
      />
      <SessionSubscription
        state={subscription}
        disabled={busy}
        desktop={desktop}
        helperText=""
      />
      <Stack spacing={0.5} data-new-session-outcome>
        <Typography variant="caption" color="text.secondary" data-new-session-continue>
          {sameAccount && samePlacement
            ? `Continue starts a fresh conversation with this agent on your next message${
              copiesWorktree ? ", with uncommitted changes" : ""
            }. This session moves to Trash.`
            : copiesWorktree
            ? "Continue moves the work to this agent on your next message, with uncommitted changes. This session moves to Trash."
            : "Continue moves the work to this agent on your next message. This session moves to Trash and keeps uncommitted changes."}
        </Typography>
        {allowFork && (
          <Typography
            variant="caption"
            color="text.secondary"
            data-new-session-fork={!samePlacement ? "unavailable" : forkMode}
          >
            {!samePlacement
              ? "Fork stays in this project and Machine."
              : forkMode === "fork"
              ? "Fork keeps both live, copying the conversation and uncommitted changes."
              : "Fork keeps both live. The new agent reads this conversation; uncommitted changes stay here."}
            {samePlacement && waitForTurn && " Available after the current turn."}
          </Typography>
        )}
      </Stack>
      <Button
        size="small"
        endIcon={
          <ExpandMore sx={{ transform: advanced ? "rotate(180deg)" : undefined }} />
        }
        aria-expanded={advanced}
        onClick={() => setAdvanced(!advanced)}
        sx={{ alignSelf: "flex-start", px: 0, textTransform: "none" }}
      >
        {advanced ? "Fewer options" : "More options"}
      </Button>
      {!placement.loading && !placement.error && !placement.project && (
        <Alert
          severity="warning"
          action={<Button onClick={() => setAdvanced(true)}>Choose project</Button>}
        >
          Choose a project in More options first.
        </Alert>
      )}
      {advanced && (
        <Stack spacing={1.5} data-context-advanced>
          <WorkspacePicker
            label="Project"
            entries={placement.projects}
            value={placement.project?.value || ""}
            onChange={placement.selectProject}
          />
          <SessionPresetPicker
            provider={provider}
            presets={presets}
            value={preset}
            onChange={setPreset}
            defaultLabel={inheritSettings && provider === session.provider
              ? "Inherit current model & reasoning"
              : "Agent default"}
            helperText={validPreset
              ? ""
              : "Choose an available preset for this agent"}
          />
          <FormControlLabel
            sx={{ mr: 0 }}
            control={
              <Checkbox
                checked={inheritSettings}
                disabled={locked}
                onChange={(_, checked) => setInheritSettings(checked)}
              />
            }
            label="Inherit compatible settings"
          />
        </Stack>
      )}
      {!validPreset && !advanced && (
        <Alert severity="warning">
          Open More options to choose a compatible preset.
        </Alert>
      )}
      {full && (
        <Alert severity="warning">
          Remove a linked session so this one can be linked.
        </Alert>
      )}
      {error && <Alert severity="error">{error}</Alert>}
      <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap" alignItems="center">
        <Button
          variant="contained"
          data-new-session-continue-run
          disabled={locked || !ready || full}
          onClick={() => void continueWork()}
        >
          {busy ? "Saving…" : "Continue on next message"}
        </Button>
        {allowFork && (
          <Button
            variant="outlined"
            data-new-session-fork-run
            disabled={locked || !ready || !samePlacement || waitForTurn}
            onClick={() => void forkWork()}
          >
            Fork now
          </Button>
        )}
        <Button disabled={busy} onClick={onCancel}>Cancel</Button>
      </Stack>
    </Stack>
  );
}

/** The last send, when it needs attention: failed, or unknown after a lost
 *  reply. */
function ContextDelivery(
  { session, context, onResolved }: {
    session: SessionMeta;
    context: SessionContext;
    onResolved: () => void;
  },
): React.JSX.Element | null {
  const [resolving, setResolving] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const receipt = context.delivery;
  if (!receipt || receipt.state === "accepted") return null;
  const unresolved = receipt.state === "preparing" || receipt.state === "unknown";
  return (
    <>
      <Alert severity={receipt.state === "failed" ? "error" : "warning"}>
        {error || receipt.error || (receipt.state === "failed"
          ? "The message was not submitted. Update these choices or clear them to keep using this agent."
          : "The previous send is being checked. Inspect the destination before doing anything again.")}
        {receipt.session_id !== session.id && (
          <Button size="small" onClick={() => openContextDestination(receipt)}>
            Open destination
          </Button>
        )}
        {unresolved && (
          <Button size="small" onClick={() => setResolving(true)}>
            Resolve interrupted send…
          </Button>
        )}
      </Alert>
      <ConfirmSheet
        open={resolving}
        onClose={() => setResolving(false)}
        title="Keep this agent after checking the send?"
        actions={
          <>
            <Button onClick={() => setResolving(false)}>Keep checking</Button>
            <Button
              color="warning"
              disabled={saving}
              onClick={() => {
                setSaving(true);
                void saveSessionContext(session.id, {
                  revision: context.revision,
                  sources: [],
                  next_agent: null,
                  resolve_operation: receipt.operation,
                })
                  .then(() => {
                    setResolving(false);
                    onResolved();
                  })
                  .catch((cause: unknown) =>
                    setError(
                      cause instanceof Error
                        ? cause.message
                        : "Could not resolve send",
                    )
                  )
                  .finally(() => setSaving(false));
              }}
            >
              Keep this agent
            </Button>
          </>
        }
      >
        <Typography>
          Inspect the destination conversation first. This clears only this
          session’s pending selection. It preserves the destination and its
          files, and does not cancel or resend any message. An uncertain send
          may have completed.
        </Typography>
      </ConfirmSheet>
    </>
  );
}

/** The collapsed summary of a saved switch, with Back until the next send. */
function PendingSwitch(
  { session, context, onBack }: {
    session: SessionMeta;
    context: SessionContext;
    onBack: () => void;
  },
): React.JSX.Element {
  const { data: accounts } = useSubscriptionAccounts();
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState("");
  const next = context.next_agent;
  const text = next
    ? `Switching to ${
      accountIdentity(accounts, next.provider, next.provider_account_id).label
    } on your next message`
    : "Linked context will be read on your next message";
  return (
    <Stack
      direction="row"
      alignItems="center"
      spacing={0.5}
      data-session-pending-switch
      sx={{ minWidth: 0 }}
    >
      <Typography
        variant="caption"
        color={failure ? "error" : "text.secondary"}
        noWrap
        sx={{ minWidth: 0 }}
        title={failure || text}
      >
        {failure || text}
      </Typography>
      <Button
        size="small"
        disabled={busy}
        data-session-switch-back
        onClick={() => {
          setBusy(true);
          setFailure("");
          void saveSessionContext(session.id, {
            revision: context.revision,
            ...withoutAgentSwitch(context, session.id),
          }).catch((error: unknown) =>
            setFailure(error instanceof Error ? error.message : "Could not go back")
          ).finally(() => {
            setBusy(false);
            onBack();
          });
        }}
        sx={{ flexShrink: 0, minWidth: 0, textTransform: "none" }}
      >
        Back
      </Button>
    </Stack>
  );
}
