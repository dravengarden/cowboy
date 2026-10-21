import { useEffect, useState } from "react";
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

export function SessionContextSettings(
  { session }: { session: SessionMeta },
): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const { data, error, reload } = useSessionContext(session.id);
  const pending = data && data.revision !== data.applied_revision;
  return (
    <Box data-session-context sx={{ minWidth: 0, pb: open ? 2 : 0 }}>
      <Button
        startIcon={<LinkOutlined />}
        endIcon={
          <ExpandMore sx={{ transform: open ? "rotate(180deg)" : undefined }} />
        }
        aria-expanded={open}
        aria-keyshortcuts="Enter"
        onClick={() => setOpen(!open)}
        sx={{ justifyContent: "flex-start", textTransform: "none" }}
      >
        Context & next agent{pending
          ? " · Pending"
          : data?.sources.length
          ? ` · ${data.sources.length} linked`
          : ""}
      </Button>
      {pending && !open && (
        <Typography variant="caption" display="block" color="text.secondary">
          {data.next_agent
            ? "A fresh agent starts on your next message. Current agent is preserved."
            : "Linked context will be read on your next message."}
        </Typography>
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
              key={session.id}
              session={session}
              observed={data}
              onSaved={() => {
                void reload();
                setOpen(false);
              }}
              onCancel={() => setOpen(false)}
            />
          )}
        </Stack>
      )}
    </Box>
  );
}

function ContextEditor(
  { session, observed, onSaved, onCancel }: {
    session: SessionMeta;
    observed: SessionContext;
    onSaved: () => void;
    onCancel: () => void;
  },
): React.JSX.Element {
  const [snapshot] = useState(observed);
  const changedElsewhere = observed.revision !== snapshot.revision ||
    observed.applied_revision !== snapshot.applied_revision ||
    observed.delivery?.operation !== snapshot.delivery?.operation;
  const [sources, setSources] = useState(snapshot.sources);
  const [fresh, setFresh] = useState(Boolean(snapshot.next_agent));
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
    }
    : null;
  const changed = sources.length !== snapshot.sources.length ||
    sources.some((id, index) => id !== snapshot.sources[index]) ||
    Boolean(nextAgent) !== Boolean(snapshot.next_agent) ||
    (agentEdited && nextAgent &&
      (Object.keys(nextAgent) as (keyof typeof nextAgent)[]).some(
        (key) => nextAgent[key] !== snapshot.next_agent?.[key],
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
      onSaved();
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
            disabled={saving || Boolean(unresolved)}
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
            <Typography variant="body2">Start with a new agent</Typography>
            <Typography variant="caption" color="text.secondary">
              {fresh ? "Starts on your next message" : "Keep the current agent"}
            </Typography>
          </Box>
        }
      />
      {fresh && (
        <>
          <WorkspacePicker
            label="Project"
            entries={placement.projects}
            value={placement.project?.value || ""}
            onChange={(value) => {
              setAgentEdited(true);
              placement.selectProject(value);
            }}
          />
          <AiInstallationPicker
            installations={placement.installations}
            value={placement.installation?.value || ""}
            onChange={(value) => {
              setAgentEdited(true);
              placement.selectInstallation(value);
            }}
            helperText={placement.error || (placement.loading
              ? "Checking available installations…"
              : "Choose where the new agent runs.")}
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
          />
          <SessionPresetPicker
            provider={provider}
            presets={presets}
            value={preset}
            onChange={(value) => {
              setAgentEdited(true);
              setPreset(value);
            }}
            helperText={validPreset
              ? "Applied before the new agent’s first prompt"
              : "Choose an available preset for this agent"}
          />
          <Typography variant="caption" color="text.secondary">
            The original conversation and files stay here. Git projects use a
            new worktree; uncommitted files are not copied.
          </Typography>
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
                {saving ? "Saving…" : "Save changes"}
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
              Cancel pending changes
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
                    onSaved();
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
