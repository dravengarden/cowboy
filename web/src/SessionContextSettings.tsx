import { useEffect, useRef, useState } from "react";
import {
  Alert,
  Box,
  Button,
  Checkbox,
  FormControlLabel,
  Stack,
  Typography,
} from "@mui/material";
import { ExpandMore, LinkOutlined } from "@mui/icons-material";
import type { SessionMeta } from "./protocol";
import { useStoreSelector } from "./store";
import { sessionExecution } from "./sessionExecution";
import { SessionContextPicker } from "./SessionContextPicker";
import {
  openContextDestination,
  saveSessionContext,
  type SessionContext,
  useSessionContext,
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
import { SessionProvenance } from "./SessionContextCue";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { accountIdentity } from "./accountIdentity";
import {
  closeSessionSettings,
  requestComposerFocus,
  SESSION_NEW_AGENT_EVENT,
  takeNewAgentRequest,
} from "./sessionSettingsOpen";

export function SessionContextSettings(
  { session, provenance = true }: { session: SessionMeta; provenance?: boolean },
): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const [startNewAgent, setStartNewAgent] = useState(false);
  const section = useRef<HTMLDivElement>(null);
  const { data, error, reload } = useSessionContext(session.id);
  const pending = data && data.revision !== data.applied_revision;
  useEffect(() => {
    const take = () => {
      if (!takeNewAgentRequest(session.id)) return;
      setStartNewAgent(true);
      setOpen(true);
    };
    take();
    document.addEventListener(SESSION_NEW_AGENT_EVENT, take);
    return () => document.removeEventListener(SESSION_NEW_AGENT_EVENT, take);
  }, [session.id]);
  useEffect(() => {
    if (open && startNewAgent) {
      section.current?.scrollIntoView({ block: "nearest" });
    }
  }, [open, startNewAgent]);
  return (
    <Box
      ref={section}
      data-session-context
      sx={{ minWidth: 0, pb: open ? 2 : 0 }}
    >
      {provenance && <SessionProvenance sessionId={session.id} />}
      <Button
        startIcon={<LinkOutlined />}
        endIcon={
          <ExpandMore sx={{ transform: open ? "rotate(180deg)" : undefined }} />
        }
        aria-expanded={open}
        aria-keyshortcuts="Enter"
        onClick={() => {
          setStartNewAgent(false);
          // Functional: a save that just closed the editor may not have
          // re-rendered yet, and a stale `open` would close it again.
          setOpen((value) => !value);
        }}
        sx={{ justifyContent: "flex-start", textTransform: "none" }}
      >
        Switch agent or link context{pending
          ? " · Pending"
          : data?.sources.length
          ? ` · ${data.sources.length} linked`
          : ""}
      </Button>
      {pending && !open && (
        <PendingSwitch session={session} context={data} onBack={() => void reload()} />
      )}
      {open && (
        <Stack spacing={1.5} sx={{ pt: 1 }}>
          {error && (
            <Alert
              severity="error"
              action={<Button onClick={() => void reload()}>Retry</Button>}
            >
              {error}
            </Alert>
          )}
          {!data && !error && (
            <Typography variant="body2">Loading context…</Typography>
          )}
          {data && (
            <ContextEditor
              key={`${session.id}:${startNewAgent}`}
              session={session}
              observed={data}
              startNewAgent={startNewAgent}
              onSaved={(staged) => {
                void reload();
                setOpen(false);
                setStartNewAgent(false);
                // A staged choice applies to the next message: go write it.
                if (staged) {
                  closeSessionSettings(session.id);
                  requestComposerFocus(session.id);
                }
              }}
              onCancel={() => {
                setOpen(false);
                setStartNewAgent(false);
              }}
            />
          )}
        </Stack>
      )}
    </Box>
  );
}

function ContextEditor(
  { session, observed, startNewAgent = false, onSaved, onCancel }: {
    session: SessionMeta;
    observed: SessionContext;
    startNewAgent?: boolean;
    onSaved: (staged: boolean) => void;
    onCancel: () => void;
  },
): React.JSX.Element {
  const [snapshot] = useState(observed);
  const changedElsewhere = observed.revision !== snapshot.revision ||
    observed.applied_revision !== snapshot.applied_revision ||
    observed.delivery?.operation !== snapshot.delivery?.operation;
  // "Continue in new agent" opens with the current session linked.
  const startFresh = startNewAgent && !snapshot.next_agent &&
    snapshot.revision === snapshot.applied_revision;
  const [sources, setSources] = useState(() =>
    startFresh && !snapshot.sources.includes(session.id)
      ? [session.id, ...snapshot.sources].slice(0, 32)
      : snapshot.sources
  );
  const [fresh, setFresh] = useState(
    Boolean(snapshot.next_agent) || startFresh,
  );
  const [advanced, setAdvanced] = useState(false);
  const [inheritSettings, setInheritSettings] = useState(
    snapshot.next_agent?.inherit_settings ?? true,
  );
  // Catalog hydration temporarily clears placement fields. Only user changes
  // to these fields can make an already-saved target dirty.
  const [agentEdited, setAgentEdited] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [resolving, setResolving] = useState(false);
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
  const placement = useProjectPlacement(fresh, machines, defaults);
  const provider = placement.installation?.provider || defaults.provider;
  const subscription = useSessionSubscription(fresh, provider);
  const desktop = useSurfaceProfile().kind === "desktop";
  const receipt = observed.delivery;
  const unresolved = receipt &&
    (receipt.state === "preparing" || receipt.state === "unknown");
  const presets =
    placement.installation?.entry.manifest.configuration.presets ?? [];
  const validPreset = !preset ||
    presets.some((entry) => `${provider}/${entry.id}` === preset);
  const account = snapshot.next_agent?.provider_account_id ||
    session.provider_account_id || "default";
  useEffect(() => {
    if (provider === defaults.provider) subscription.select(account);
  }, [provider, fresh, account]);
  const nextAgent = fresh
    ? {
      provider,
      provider_account_id: subscription.value,
      machine_id: placement.machineId,
      runtime_machine_id: placement.runtimeMachineId || null,
      cwd: placement.project?.projectId || "",
      preset: preset || null,
      inherit_settings: inheritSettings,
    }
    : null;
  // Model, reasoning and permissions are live configuration of this agent.
  // Only another Provider, subscription, project or Machine needs a new one.
  const sameAgent = Boolean(nextAgent) &&
    nextAgent?.provider === session.provider &&
    nextAgent.provider_account_id ===
      (session.provider_account_id || "default") &&
    nextAgent.cwd === (session.workspace_id || "") &&
    nextAgent.machine_id === (execution.machineId || null) &&
    (nextAgent.runtime_machine_id || session.machine_id) === session.machine_id;
  const changed = sources.length !== snapshot.sources.length ||
    sources.some((id, index) => id !== snapshot.sources[index]) ||
    Boolean(nextAgent) !== Boolean(snapshot.next_agent) ||
    (agentEdited && nextAgent &&
      (Object.keys(nextAgent) as (keyof typeof nextAgent)[]).some(
        (key) =>
          nextAgent[key] !==
            (key === "inherit_settings"
              ? snapshot.next_agent?.inherit_settings ?? true
              : snapshot.next_agent?.[key]),
      ));
  const pending = snapshot.revision !== snapshot.applied_revision;
  const locked = saving || changedElsewhere || Boolean(unresolved);
  const save = async (clear = false) => {
    if (locked) return;
    setSaving(true);
    setError("");
    try {
      await saveSessionContext(session.id, {
        revision: snapshot.revision,
        sources: clear ? [] : sources,
        next_agent: clear ? null : nextAgent,
      });
      onSaved(!clear && (Boolean(nextAgent) || sources.length > 0));
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not save context",
      );
    } finally {
      setSaving(false);
    }
  };
  return (
    <Stack spacing={1.5} data-context-editor>
      {changedElsewhere && (
        <Alert severity="warning">
          Context changed on another device. Close and reopen this section to
          review it; your choices have not been applied.
        </Alert>
      )}
      {receipt && (
        <Alert
          severity={receipt.state === "failed"
            ? "error"
            : unresolved
            ? "warning"
            : "info"}
        >
          {receipt.state === "accepted"
            ? "The last message was accepted."
            : receipt.error || (receipt.state === "failed"
              ? "The message was not submitted. Update these choices or clear them to keep using this agent."
              : "The previous send is being checked. Inspect the destination before doing anything again.")}
          {receipt.session_id !== session.id && (
            <Button
              size="small"
              onClick={() => openContextDestination(receipt)}
            >
              Open destination
            </Button>
          )}
          {unresolved && (
            <Button
              size="small"
              onClick={() => setResolving(true)}
            >
              Resolve interrupted send…
            </Button>
          )}
        </Alert>
      )}
      <FormControlLabel
        sx={{ mr: 0 }}
        control={
          <Checkbox
            checked={fresh}
            disabled={locked}
            onChange={(_, checked) => {
              setFresh(checked);
              if (checked) {
                setSources((ids) =>
                  ids.includes(session.id)
                    ? ids
                    : [session.id, ...ids].slice(0, 32)
                );
              } else setSources((ids) => ids.filter((id) => id !== session.id));
            }}
          />
        }
        label={
          <Box>
            <Typography variant="body2">Continue with a new agent</Typography>
            <Typography variant="caption" color="text.secondary">
              {fresh
                ? "Opens beside this session on your next message"
                : "Keep the current agent"}
            </Typography>
          </Box>
        }
      />
      {fresh && (
        <>
          <AiInstallationPicker
            installations={placement.installations}
            value={placement.installation?.value || ""}
            onChange={(value) => {
              setAgentEdited(true);
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
            state={{
              ...subscription,
              select: (value) => {
                setAgentEdited(true);
                subscription.select(value);
              },
            }}
            disabled={saving}
            desktop={desktop}
            helperText=""
          />
          <Box data-context-inheritance>
            <Typography variant="body2" color="text.secondary">
              {placement.project?.name || session.workspace_name ||
                "Current project"}
              {" · Same sidebar folder"}
              {inheritSettings ? " · Inherit settings" : " · Agent defaults"}
            </Typography>
            <Typography
              variant="caption"
              color="text.secondary"
              component="p"
              data-context-files
            >
              Settings are inherited, files are not: Git projects start a new
              worktree, and uncommitted changes stay with this session. The new
              agent is asked to inspect them before editing.
            </Typography>
            <Button
              size="small"
              endIcon={
                <ExpandMore
                  sx={{ transform: advanced ? "rotate(180deg)" : undefined }}
                />
              }
              aria-expanded={advanced}
              onClick={() => setAdvanced(!advanced)}
              sx={{ px: 0, textTransform: "none" }}
            >
              {advanced ? "Fewer options" : "More options"}
            </Button>
          </Box>
          {sameAgent && (
            <Typography
              variant="caption"
              color="text.secondary"
              data-context-same-agent
            >
              Same Provider, subscription, project and Machine. Model, reasoning
              and permissions change in this agent from Run Configuration; a new
              agent only starts a fresh conversation.
            </Typography>
          )}
          {!placement.loading && !placement.error && !placement.project && (
            <Alert
              severity="warning"
              action={
                <Button onClick={() => setAdvanced(true)}>
                  Choose project
                </Button>
              }
            >
              Choose a project in More options before saving.
            </Alert>
          )}
          {advanced && (
            <Stack spacing={1.5} data-context-advanced>
              <WorkspacePicker
                label="Project"
                entries={placement.projects}
                value={placement.project?.value || ""}
                onChange={(value) => {
                  setAgentEdited(true);
                  placement.selectProject(value);
                }}
              />
              <FormControlLabel
                sx={{ mr: 0 }}
                control={
                  <Checkbox
                    checked={inheritSettings}
                    disabled={locked}
                    onChange={(_, checked) => {
                      setAgentEdited(true);
                      setInheritSettings(checked);
                    }}
                  />
                }
                label="Inherit compatible settings"
              />
              <SessionPresetPicker
                provider={provider}
                presets={presets}
                value={preset}
                onChange={(value) => {
                  setAgentEdited(true);
                  setPreset(value);
                }}
                defaultLabel={inheritSettings && provider === session.provider
                  ? "Inherit current model & reasoning"
                  : "Agent default"}
                helperText={validPreset
                  ? ""
                  : "Choose an available preset for this agent"}
              />
              <Typography variant="caption" color="text.secondary">
                {inheritSettings
                  ? "Copies compatible Provider settings and session overrides. Memory and Calls follow defaults where unset. A preset overrides model and reasoning."
                  : "Uses the selected agent’s defaults."}
              </Typography>
            </Stack>
          )}
          {!validPreset && !advanced && (
            <Alert severity="warning">
              Open More options to choose a compatible preset.
            </Alert>
          )}
        </>
      )}
      <SessionContextPicker
        value={sources}
        onChange={setSources}
        disabled={saving || Boolean(unresolved)}
        exclude={fresh ? undefined : session.id}
      />
      {error && <Alert severity="error">{error}</Alert>}
      {changed
        ? (
          <Stack spacing={1} data-context-changes>
            <Typography variant="caption" color="text.secondary">
              Unsaved changes · Apply on your next message
            </Typography>
            <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap">
              <Button
                variant="contained"
                disabled={locked || (fresh &&
                  (!placement.ready || !subscription.allowed || !validPreset))}
                onClick={() => void save()}
              >
                {saving
                  ? "Saving…"
                  : nextAgent && !sameAgent &&
                      (nextAgent.provider !== session.provider ||
                        nextAgent.provider_account_id !==
                          (session.provider_account_id || "default"))
                  ? "Switch"
                  : "Save changes"}
              </Button>
              <Button disabled={saving} onClick={onCancel}>Discard</Button>
            </Stack>
          </Stack>
        )
        : pending && !unresolved
        ? (
          <Stack spacing={0.5}>
            <Typography variant="caption" color="text.secondary">
              Saved · Applies on your next message
            </Typography>
            <Button
              disabled={locked}
              onClick={() => void save(true)}
              sx={{ alignSelf: "flex-start" }}
            >
              {snapshot.next_agent ? "Back to current agent" : "Cancel linked context"}
            </Button>
          </Stack>
        )
        : null}
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
                if (!receipt) return;
                setSaving(true);
                void saveSessionContext(session.id, {
                  revision: snapshot.revision,
                  sources: [],
                  next_agent: null,
                  resolve_operation: receipt.operation,
                })
                  .then(() => {
                    setResolving(false);
                    onSaved(false);
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
    </Stack>
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
            sources: [],
            next_agent: null,
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
