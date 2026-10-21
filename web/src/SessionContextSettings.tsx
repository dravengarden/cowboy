import { useEffect, useState } from "react";
import {
  Alert,
  Box,
  Button,
  Checkbox,
  FormControlLabel,
  MenuItem,
  Stack,
  TextField,
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
    <Box data-session-context sx={{ minWidth: 0 }}>
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
            />
          )}
        </Stack>
      )}
    </Box>
  );
}

function ContextEditor(
  { session, observed, onSaved }: {
    session: SessionMeta;
    observed: SessionContext;
    onSaved: () => void;
  },
): React.JSX.Element {
  const [snapshot] = useState(observed);
  const changedElsewhere = observed.revision !== snapshot.revision ||
    observed.applied_revision !== snapshot.applied_revision ||
    observed.delivery?.operation !== snapshot.delivery?.operation;
  const [sources, setSources] = useState(snapshot.sources);
  const [fresh, setFresh] = useState(Boolean(snapshot.next_agent));
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
  const save = async (clear = false) => {
    if (saving || unresolved) return;
    setSaving(true);
    setError("");
    try {
      await saveSessionContext(session.id, {
        revision: snapshot.revision,
        sources: clear ? [] : sources,
        next_agent: !clear && fresh
          ? {
            provider,
            provider_account_id: subscription.value,
            machine_id: placement.machineId,
            runtime_machine_id: placement.runtimeMachineId || null,
            cwd: placement.project?.projectId || "",
            preset: preset || null,
          }
          : null,
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
        label="Start with a new agent"
      />
      {fresh && (
        <>
          <Typography variant="body2" color="text.secondary">
            Choose the next Provider and account. Your current agent stays until
            you send.
          </Typography>
          <WorkspacePicker
            label="Project"
            entries={placement.projects}
            value={placement.project?.value || ""}
            onChange={placement.selectProject}
          />
          <AiInstallationPicker
            installations={placement.installations}
            value={placement.installation?.value || ""}
            onChange={placement.selectInstallation}
            helperText={placement.error || (placement.loading
              ? "Checking available installations…"
              : "Choose where the new agent runs.")}
          />
          <SessionSubscription
            state={subscription}
            disabled={saving}
            desktop={desktop}
          />
          <TextField
            select
            fullWidth
            label="Model and reasoning preset"
            value={preset}
            onChange={(event) => setPreset(event.target.value)}
            error={!validPreset}
          >
            <MenuItem value="">Agent default</MenuItem>
            {!validPreset && (
              <MenuItem disabled value={preset}>
                {preset} · Unavailable
              </MenuItem>
            )}
            {presets.map((entry) => (
              <MenuItem key={entry.id} value={`${provider}/${entry.id}`}>
                {entry.name}
              </MenuItem>
            ))}
          </TextField>
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
      <Typography variant="caption" color="text.secondary">
        You can change or clear this selection before sending.
      </Typography>
      {error && <Alert severity="error">{error}</Alert>}
      <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap">
        <Button
          variant="contained"
          disabled={saving || changedElsewhere || Boolean(unresolved) ||
            (fresh &&
              (!placement.ready || !subscription.allowed || !validPreset))}
          onClick={() => void save()}
        >
          Use on next message
        </Button>
        <Button
          disabled={saving || changedElsewhere || Boolean(unresolved)}
          onClick={() => void save(true)}
        >
          Clear selection
        </Button>
      </Stack>
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
