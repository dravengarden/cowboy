import { useEffect, useRef, useState } from "react";
import { Alert, Box, Button, Stack, type SxProps, type Theme, Typography } from "@mui/material";
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
import { ForkConfiguration, type ForkConfig, useForkTargetOptions } from "./ForkConfiguration";
import { ForkOutcome } from "./ForkOutcome";
import { NewSessionActions } from "./NewSessionActions";
import { SessionSubscription, useSessionSubscription } from "./SessionSubscription";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { ConfirmSheet } from "./Sheet";
import { SessionProvenance } from "./SessionContextCue";
import { useHistorySession } from "./sessionHistory";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { accountIdentity } from "./accountIdentity";
import { duplicateMode, duplicateSession, forkAvailability, forkSession } from "./sessionFork";
import { requestPickSession } from "./sessionPickRequest";
import {
  closeSessionSettings,
  requestComposerFocus,
  SESSION_NEW_FROM_EVENT,
  takeNewFromRequest,
} from "./sessionSettingsOpen";
import { newUuid } from "./uuid";
import { useTargetPreset } from "./newSessionPreset";

/** Three questions, one place each. How this agent runs lives in the Agent
 *  block above. Here: what the next message can read (Link sessions), and
 *  where the work goes (Replace or fork). */
export function SessionContextSettings({
  session,
  provenance = true,
  allowFork = true,
  inlineLinks = false,
}: {
  session: SessionMeta;
  provenance?: boolean;
  /** An empty session has no conversation to branch. */
  allowFork?: boolean;
  /** Desktop workbench: Replace or fork first, then the session tree in
   *  place, filling the rest of the pane. */
  inlineLinks?: boolean;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<string[] | null>(null);
  const [reveal, setReveal] = useState(false);
  // Remounting the form takes a fresh snapshot of the context it replaces.
  const [formGeneration, setFormGeneration] = useState(0);
  const actions = useRef<HTMLDivElement>(null);
  const { data, error, reload } = useSessionContext(session.id);
  const handedOff = useHistorySession(session.id);
  useEffect(() => {
    const take = () => {
      if (!takeNewFromRequest(session.id)) return;
      setOpen(true);
      setReveal(true);
    };
    take();
    document.addEventListener(SESSION_NEW_FROM_EVENT, take);
    return () => document.removeEventListener(SESSION_NEW_FROM_EVENT, take);
  }, [session.id]);
  // Requested from the session menu: land on the form, once it exists. The
  // settings surface and the form's pickers settle after the first frame.
  const ready = open && Boolean(data);
  useEffect(() => {
    if (!reveal || !ready) return undefined;
    const show = () => {
      const target = actions.current;
      if (!target) return;
      // On the Desktop workbench (`inlineLinks`), Replace or fork already opens
      // at the top of the Context pane unless a provenance/delivery block sits
      // above it — avoid forcing a synchronous scrollIntoView layout pass.
      if (inlineLinks && target.parentElement?.firstElementChild === target &&
          target.parentElement.previousElementSibling === null) return;
      target.scrollIntoView({ block: "start" });
    };
    const frame = requestAnimationFrame(show);
    const settled = setTimeout(() => {
      show();
      setReveal(false);
    }, 400);
    return () => {
      cancelAnimationFrame(frame);
      clearTimeout(settled);
    };
  }, [inlineLinks, reveal, ready]);
  const onSaved = () => {
    void reload();
    setOpen(false);
  };
  // The injection ends with its turn, without a context announcement.
  useEffect(() => {
    if (open) void reload();
  }, [open, reload, session.status]);
  const stagedAgent = Boolean(data?.next_agent) && data?.revision !== data?.applied_revision;
  return (
    <Box data-session-context sx={{ minWidth: 0, pb: open ? 2 : 0 }}>
      {provenance && <SessionProvenance sessionId={session.id} />}
      {/* A handed-off session's work moved on: a further switch or new links
          from here would split it, but a fork with chosen options stays. */}
      {/* Flex gap, not Stack margins: the workbench wrapper and the form's
          target group are `display: contents` off the Desktop workbench, and
          margins on a box-less wrapper vanish, collapsing the fields together. */}
      <Stack spacing={1.5} useFlexGap>
        {error && (
          <Alert severity="error" action={<Button onClick={() => void reload()}>Retry</Button>}>
            {error}
          </Alert>
        )}
        {data && !handedOff && (
          <>
            <ContextDelivery session={session} context={data} onResolved={onSaved} />
            {!inlineLinks && (
              <LinkedSessions
                session={session}
                context={data}
                draft={draft}
                onDraft={setDraft}
                onSettled={reload}
              />
            )}
          </>
        )}
        <Box ref={actions} data-session-new-from-section>
          <Button
            startIcon={<ForkRightOutlined />}
            endIcon={<ExpandMore sx={{ transform: open ? "rotate(180deg)" : undefined }} />}
            aria-expanded={open}
            aria-keyshortcuts="Enter"
            disabled={!data}
            // Functional: a save that just closed the form may not have
            // re-rendered yet, and a stale `open` would close it again.
            onClick={() => setOpen((value) => !value)}
            sx={{ justifyContent: "flex-start", textTransform: "none" }}
          >
            {handedOff ? "Fork" : "Replace or fork"}
          </Button>
          {data && stagedAgent && !open && !handedOff && (
            <PendingSwitch session={session} context={data} onBack={() => void reload()} />
          )}
        </Box>
        <Box
          data-context-workbench={inlineLinks && open ? "split" : undefined}
          sx={contextWorkbenchSx(inlineLinks, open)}
        >
        {data && open && (
          <NewSessionForm
            key={`${session.id}:${formGeneration}`}
            session={session}
            observed={data}
            sources={draft ?? pendingSources(data)}
            allowFork={allowFork && !session.system && !isManagedChild(session)}
            forkOnly={handedOff}
            onContinued={() => {
              onSaved();
              // A staged choice applies to the next message: go write it.
              closeSessionSettings(session.id);
              requestComposerFocus(session.id);
            }}
            onForked={() => setOpen(false)}
            onRestart={() => setFormGeneration((value) => value + 1)}
          />
        )}
        {data && inlineLinks && !handedOff && (
          <LinkedSessions
            session={session}
            context={data}
            draft={draft}
            onDraft={setDraft}
            onSettled={reload}
            inline
          />
        )}
        </Box>
      </Stack>
    </Box>
  );
}

/** Bring the project field into reach: Desktop shows it, touch discloses it. */
function chooseProject(desktop: boolean, disclose: (open: boolean) => void): void {
  if (!desktop) return disclose(true);
  const field = document.querySelector<HTMLElement>("[data-fork-project] [role='combobox'], [data-fork-project] input");
  field?.scrollIntoView({ block: "nearest" });
  field?.focus();
}

/** Desktop: the installation and subscription side by side. */
const targetGridSx = { display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))", gap: 1.25 };

/** Desktop workbench: the new agent's form beside the session tree, held in
 *  view while the tree scrolls. Elsewhere the wrapper adds no box. */
function contextWorkbenchSx(inlineLinks: boolean, open: boolean): SxProps<Theme> {
  if (!inlineLinks) return { display: "contents" };
  return {
    display: "grid",
    gap: 2,
    gridTemplateColumns: "minmax(0, 1fr)",
    alignItems: "start",
    "@container run-config-pane (min-width: 760px)": open
      ? {
          gridTemplateColumns: "minmax(0, 1.15fr) minmax(0, 1fr)",
          "& > [data-new-session-editor]": { position: "sticky", top: 0 },
        }
      : {},
  };
}

/** What the next prompt will read, whichever agent receives it. Like an
 *  attachment: each change applies at once and nothing runs until that prompt
 *  is sent, so there is nothing to save. A fork leaves this selection here. */
function LinkedSessions({
  session,
  context,
  draft,
  onDraft,
  onSettled,
  inline = false,
}: {
  session: SessionMeta;
  context: SessionContext;
  draft: string[] | null;
  onDraft: (draft: string[] | null) => void;
  onSettled: () => Promise<unknown>;
  inline?: boolean;
}): React.JSX.Element {
  const [error, setError] = useState("");
  const [recovering, setRecovering] = useState(false);
  const latest = useRef<string[] | null>(null);
  const applying = useRef(false);
  const revision = useRef(context.revision);
  if (!applying.current) revision.current = context.revision;
  const unresolved =
    context.delivery?.state === "preparing" || context.delivery?.state === "unknown";
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
        inline={inline}
      />
      {error && <Alert severity="error">{error}</Alert>}
    </Stack>
  );
}

type PendingSettingsFork =
  | { mode: "fork"; operation: string; config: ForkConfig }
  | {
      mode: "linked";
      operation: string;
      body: {
        provider: string;
        provider_account_id: string;
        preset: string | null;
        inherit_settings: boolean;
        config: ForkConfig;
        include_uncommitted?: boolean;
      };
    };

function buildSettingsForkAttempt(input: {
  forkMode: "fork" | "linked";
  provider: string;
  account: string;
  preset: string | undefined;
  inheritSettings: boolean;
  overrides: ForkConfig;
  effectiveIncludeUncommitted: boolean | undefined;
}): PendingSettingsFork {
  return input.forkMode === "fork"
    ? { mode: "fork", operation: `duplicate-${newUuid()}`, config: input.overrides }
    : {
        mode: "linked",
        operation: `duplicate-${newUuid()}`,
        body: {
          provider: input.provider,
          provider_account_id: input.account,
          preset: input.preset || null,
          inherit_settings: input.inheritSettings,
          config: input.overrides,
          ...(input.effectiveIncludeUncommitted !== undefined
            ? { include_uncommitted: input.effectiveIncludeUncommitted }
            : {}),
        },
      };
}

function submitSettingsFork(sessionId: string, attempt: PendingSettingsFork) {
  return attempt.mode === "fork"
    ? forkSession(sessionId, { operation: attempt.operation, config: attempt.config })
    : duplicateSession(sessionId, attempt.body, attempt.operation);
}

/** Where the work goes: one target, then either move the work there or keep
 *  both lines live. How history travels follows from the target. */
function NewSessionForm({
  session,
  observed,
  sources,
  allowFork,
  forkOnly,
  onContinued,
  onForked,
  onRestart,
}: {
  session: SessionMeta;
  observed: SessionContext;
  sources: string[];
  allowFork: boolean;
  /** History: Replace would split the work that moved on; a fork keeps both. */
  forkOnly: boolean;
  onContinued: () => void;
  onForked: () => void;
  /** Start over from the latest context. */
  onRestart: () => void;
}): React.JSX.Element {
  const [snapshot] = useState(observed);
  // Links are edited beside this form; only what it replaces is a conflict.
  // Another device, or another settings surface on this one (the Composer
  // panel and an empty session's start panel each mount this form).
  const changedElsewhere =
    JSON.stringify(observed.next_agent) !== JSON.stringify(snapshot.next_agent) ||
    observed.delivery?.operation !== snapshot.delivery?.operation;
  // Nothing chosen here yet: follow the change instead of asking for a
  // reopen. Once the form was used, its choices wait for an explicit Reload.
  const [touched, setTouched] = useState(false);
  const [advanced, setAdvanced] = useState(false);
  const [forkOpts, setForkOpts] = useState(() => ({
    inherit: snapshot.next_agent?.inherit_settings ?? true,
    uncommitted: session.status !== "busy" && session.status !== "starting",
  }));
  const inheritSettings = forkOpts.inherit;
  const setInheritSettings = (v: boolean) => setForkOpts((o) => ({ ...o, inherit: v }));
  const copyUncommitted = forkOpts.uncommitted;
  const setCopyUncommitted = (v: boolean) => setForkOpts((o) => ({ ...o, uncommitted: v }));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [overrides, setOverrides] = useState<ForkConfig>({});
  const liveOptions = useStoreSelector((state) => state.configOptions.get(session.id));
  const machines = useStoreSelector((state) => state.machines);
  const execution = sessionExecution(session);
  const defaults = {
    project: snapshot.next_agent?.cwd || session.workspace_id || null,
    machine: snapshot.next_agent?.machine_id || execution.machineId || null,
    runtime_machine: snapshot.next_agent?.runtime_machine_id || session.machine_id || null,
    provider: snapshot.next_agent?.provider || session.provider,
  };
  const placement = useProjectPlacement(true, machines, defaults);
  const provider = placement.installation?.provider || defaults.provider;
  const subscription = useSessionSubscription(true, provider);
  const desktop = useSurfaceProfile().kind === "desktop";
  const unresolved =
    observed.delivery?.state === "preparing" || observed.delivery?.state === "unknown";
  const presets = placement.installation?.entry.manifest.configuration.presets ?? [];
  const [preset, setPreset] = useTargetPreset(snapshot.next_agent, provider, session.provider, presets);
  const validPreset = !preset || presets.some((entry) => `${provider}/${entry.id}` === preset);
  const sourceAccount = session.provider_account_id || "default";
  const account = snapshot.next_agent?.provider_account_id || sourceAccount;
  useEffect(() => {
    if (provider === defaults.provider) subscription.select(account);
  }, [provider, account]);
  const target = useForkTargetOptions(session.id, { provider, account: subscription.value, preset, inherit: inheritSettings },
    provider === session.provider ? liveOptions : undefined, Boolean(subscription.value));
  const nextAgent = {
    provider,
    provider_account_id: subscription.value,
    machine_id: placement.machineId,
    runtime_machine_id: placement.runtimeMachineId || null,
    cwd: placement.project?.projectId || "",
    preset: preset || null,
    inherit_settings: inheritSettings,
    ...(Object.keys(overrides).length > 0 ? { config: overrides } : {}),
  };
  const sameAccount = provider === session.provider && subscription.value === sourceAccount;
  // Worktrees live on the execution Machine, per project.
  const sameHome =
    nextAgent.cwd === (session.workspace_id || "") &&
    nextAgent.machine_id === (execution.machineId || null);
  const targetMachine = machines.find((machine) => machine.id === nextAgent.machine_id);
  const copiesWorktree = sameHome && Boolean(targetMachine?.capabilities?.native_fork);
  const supportsWorkspaceMode = copiesWorktree && Boolean(targetMachine?.capabilities?.fork_workspace_mode);
  // A fork stays in this session's project and Machines.
  const samePlacement =
    sameHome && (nextAgent.runtime_machine_id || session.machine_id) === session.machine_id;
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
  // The server refuses a new agent while prompts wait here, so the form says
  // so first and follows the live queue and status instead of answering a
  // press with an error.
  const queued = useStoreSelector((state) => state.queues.get(session.id)?.length ?? 0);
  const waitForTurn = forkMode === "fork" && fork === "busy";
  const submittedRunning = observed.injection_pending === true && session.status === "busy";
  const replaceWaits = queued > 0 || submittedRunning;
  const forkWaits = waitForTurn || (forkMode === "fork" && queued > 0);
  const sourceBusy = session.status === "busy" || session.status === "starting";
  const effectiveIncludeUncommitted = supportsWorkspaceMode
    ? sourceBusy
      ? false
      : copyUncommitted
    : undefined;
  const choiceSignature = JSON.stringify([provider, subscription.value, preset, inheritSettings, overrides, copyUncommitted]);
  const pendingForkRef = useRef<{ signature: string; attempt: PendingSettingsFork } | null>(null);
  // The first send needs this session as a source unless it was removed from
  // an already staged continuation.
  const continueSources =
    snapshot.next_agent || sources.includes(session.id) ? sources : [session.id, ...sources];
  const full = continueSources.length > observed.max_sources;
  const ready = placement.ready && subscription.allowed && validPreset;
  const locked = busy || changedElsewhere || unresolved;
  useEffect(() => {
    if (changedElsewhere && !touched && !busy) onRestart();
  }, [changedElsewhere, touched, busy]);
  // A refusal describes the moment it was made.
  useEffect(() => setError(""), [queued, session.status]);
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
      if (pendingForkRef.current?.signature !== choiceSignature) {
        pendingForkRef.current = {
          signature: choiceSignature,
          attempt: buildSettingsForkAttempt({ forkMode, provider, account: subscription.value, preset, inheritSettings, overrides, effectiveIncludeUncommitted }),
        };
      }
      const receipt = await submitSettingsFork(session.id, pendingForkRef.current.attempt);
      pendingForkRef.current = null;
      onForked();
      closeSessionSettings(session.id);
      requestPickSession(receipt.session_id, receipt.session ?? undefined);
    }, "Fork failed");
  const canReplace = !forkOnly && !locked && ready && !full && !replaceWaits;
  const canFork = allowFork && !locked && ready && samePlacement && !forkWaits;
  return (
    <Stack
      spacing={1.5}
      useFlexGap
      data-new-session-editor
      // React events bubble from the pickers' menu portals too.
      onClickCapture={() => setTouched(true)}
      onKeyDownCapture={(event) => {
        if (event.key === "Enter" || event.key === " ") setTouched(true);
      }}
    >
      {changedElsewhere && (
        <Alert
          severity="warning"
          data-new-session-stale
          action={
            <Button color="inherit" size="small" onClick={onRestart}>
              Reload
            </Button>
          }
        >
          This session's context changed elsewhere. Reload to review it; your choices here have not
          been applied.
        </Alert>
      )}
      <Box data-new-session-target sx={desktop ? targetGridSx : { display: "contents" }}>
      <AiInstallationPicker
        installations={placement.installations}
        value={placement.installation?.value || ""}
        onChange={(value) => {
          if (placement.installations.find((item) => item.value === value)?.provider !== provider) {
            setPreset(undefined);
            setOverrides({});
          }
          placement.selectInstallation(value);
        }}
        helperText={
          placement.error || (placement.loading ? "Checking available installations…" : "")
        }
      />
      <SessionSubscription state={subscription} disabled={busy} desktop={desktop} helperText="" />
      </Box>
      <ForkOutcome forkOnly={forkOnly} allowFork={allowFork} sameAccount={sameAccount} samePlacement={samePlacement} copiesWorktree={copiesWorktree} supportsWorkspaceMode={supportsWorkspaceMode} forkMode={forkMode} waitForTurn={waitForTurn && queued === 0} sourceBusy={sourceBusy} copyUncommitted={copyUncommitted} onCopyUncommitted={setCopyUncommitted} />
      {!placement.loading && !placement.error && !placement.project && (
        <Alert
          severity="warning"
          action={<Button onClick={() => chooseProject(desktop, setAdvanced)}>Choose project</Button>}
        >
          {desktop ? "Choose a project below first." : "Choose a project in More options first."}
        </Alert>
      )}
      <ForkConfiguration
        disclosure={
          <Button
            size="small"
            endIcon={<ExpandMore sx={{ transform: advanced ? "rotate(180deg)" : undefined }} />}
            aria-expanded={advanced}
            onClick={() => setAdvanced(!advanced)}
            sx={{ alignSelf: "flex-start", px: 0, textTransform: "none" }}
          >
            {advanced ? "Fewer options" : "More options"}
          </Button>
        }
        provider={provider}
        presets={presets}
        preset={preset}
        onPreset={setPreset}
        inherit={inheritSettings}
        onInherit={setInheritSettings}
        sameProvider={provider === session.provider}
        options={target.options}
        loading={target.loading}
        optionsError={target.error}
        overrides={overrides}
        onOverrides={setOverrides}
        advanced={advanced}
        locked={locked}
        project={
          <WorkspacePicker
            label="Project"
            entries={placement.projects}
            value={placement.project?.value || ""}
            onChange={placement.selectProject}
          />
        }
      />
      {!validPreset && <Alert severity="warning">Choose an available preset for this Agent.</Alert>}
      {full && !forkOnly && <Alert severity="warning">Remove a linked session so this one can be linked.</Alert>}
      {submittedRunning && queued === 0 && !forkOnly && (
        <Typography variant="caption" color="text.secondary" data-new-session-wait>
          Replace is available once the submitted message finishes.
        </Typography>
      )}
      {queued > 0 && (!forkOnly || forkMode === "fork") && (
        <Typography variant="caption" color="text.secondary" data-new-session-wait>
          {forkOnly
            ? "Fork is"
            : allowFork && forkMode === "fork" && samePlacement
            ? "Replace and Fork are"
            : "Replace is"}{" "}
          available once{" "}
          {queued === 1 ? "the queued message is" : `the ${queued} queued messages are`} sent or
          removed.
        </Typography>
      )}
      {error && <Alert severity="error">{error}</Alert>}
      <NewSessionActions
        desktop={desktop}
        forkOnly={forkOnly}
        allowFork={allowFork}
        canReplace={canReplace}
        canFork={canFork}
        busy={busy}
        onReplace={continueWork}
        onFork={forkWork}
      />
    </Stack>
  );
}

/** The last send, when it needs attention: failed, or unknown after a lost
 *  reply. */
function ContextDelivery({
  session,
  context,
  onResolved,
}: {
  session: SessionMeta;
  context: SessionContext;
  onResolved: () => void;
}): React.JSX.Element | null {
  const [resolving, setResolving] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const receipt = context.delivery;
  if (!receipt || receipt.state === "accepted") return null;
  const unresolved = receipt.state === "preparing" || receipt.state === "unknown";
  return (
    <>
      <Alert severity={receipt.state === "failed" ? "error" : "warning"}>
        {error ||
          receipt.error ||
          (receipt.state === "failed"
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
                    setError(cause instanceof Error ? cause.message : "Could not resolve send"),
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
          Inspect the destination conversation first. This clears only this session’s pending
          selection. It preserves the destination and its files, and does not cancel or resend any
          message. An uncertain send may have completed.
        </Typography>
      </ConfirmSheet>
    </>
  );
}

/** The collapsed summary of a saved switch, with Back until the next send. */
function PendingSwitch({
  session,
  context,
  onBack,
}: {
  session: SessionMeta;
  context: SessionContext;
  onBack: () => void;
}): React.JSX.Element {
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
          })
            .catch((error: unknown) =>
              setFailure(error instanceof Error ? error.message : "Could not go back"),
            )
            .finally(() => {
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
