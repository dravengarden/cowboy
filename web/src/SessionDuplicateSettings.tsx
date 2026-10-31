import { useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  Box,
  Button,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { ContentCopyOutlined, ExpandMore } from "@mui/icons-material";
import type { ConfigOption, SessionMeta } from "./protocol";
import { useStoreSelector } from "./store";
import { sessionExecution } from "./sessionExecution";
import { useProjectPlacement } from "./useProjectPlacement";
import { AiInstallationPicker } from "./AiInstallationPicker";
import { SessionPresetPicker } from "./SessionPresetPicker";
import { SessionSubscription, useSessionSubscription } from "./SessionSubscription";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { isCoreOption } from "./SessionStartSettings";
import {
  duplicateMode,
  duplicateSession,
  forkAvailability,
  forkSession,
} from "./sessionFork";
import { requestPickSession } from "./sessionPickRequest";
import {
  closeSessionSettings,
  SESSION_DUPLICATE_EVENT,
  takeDuplicateRequest,
} from "./sessionSettingsOpen";
import { newUuid } from "./uuid";

const NO_OPTIONS: readonly ConfigOption[] = [];

/** Duplicate this session in Session Settings: the same agent copies the
 *  whole native conversation (model and reasoning may change); another
 *  subscription or Provider starts a new agent that reads this one as linked
 *  context. The source is never changed. */
export function SessionDuplicateSettings(
  { session }: { session: SessionMeta },
): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const section = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const take = () => {
      if (takeDuplicateRequest(session.id)) setOpen(true);
    };
    take();
    document.addEventListener(SESSION_DUPLICATE_EVENT, take);
    return () => document.removeEventListener(SESSION_DUPLICATE_EVENT, take);
  }, [session.id]);
  useEffect(() => {
    if (open) section.current?.scrollIntoView({ block: "nearest" });
  }, [open]);
  return (
    <Box ref={section} data-session-duplicate sx={{ minWidth: 0, pb: open ? 2 : 0 }}>
      <Button
        startIcon={<ContentCopyOutlined />}
        endIcon={<ExpandMore sx={{ transform: open ? "rotate(180deg)" : undefined }} />}
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        sx={{ justifyContent: "flex-start", textTransform: "none" }}
      >
        Duplicate…
      </Button>
      {open && <DuplicateEditor session={session} onDone={() => setOpen(false)} />}
    </Box>
  );
}

function DuplicateEditor(
  { session, onDone }: { session: SessionMeta; onDone: () => void },
): React.JSX.Element {
  const machines = useStoreSelector((state) => state.machines);
  const liveOptions = useStoreSelector((state) =>
    state.configOptions.get(session.id) ?? NO_OPTIONS
  );
  const execution = sessionExecution(session);
  const defaults = {
    project: session.workspace_id || null,
    machine: execution.machineId || null,
    runtime_machine: session.machine_id || null,
    provider: session.provider,
  };
  const placement = useProjectPlacement(true, machines, defaults);
  const provider = placement.installation?.provider || session.provider;
  const subscription = useSessionSubscription(true, provider);
  const sourceAccount = session.provider_account_id || "default";
  useEffect(() => {
    if (provider === session.provider) subscription.select(sourceAccount);
  }, [provider, sourceAccount]);
  const desktop = useSurfaceProfile().kind === "desktop";
  const fork = forkAvailability(session, machines);
  const sameAgent = provider === session.provider &&
    subscription.value === sourceAccount;
  const mode = duplicateMode({
    sourceProvider: session.provider,
    sourceAccount,
    provider,
    account: subscription.value,
    forkAvailable: fork !== null,
  });
  // Same Provider: the copy starts from this agent's live model and reasoning.
  const coreOptions = useMemo(
    () => sameAgent ? liveOptions.filter(isCoreOption) : [],
    [sameAgent, liveOptions],
  );
  const [overrides, setOverrides] = useState<Record<string, string | boolean>>({});
  const [preset, setPreset] = useState("");
  const presets = placement.installation?.entry.manifest.configuration.presets ?? [];
  const validPreset = !preset ||
    presets.some((entry) => `${provider}/${entry.id}` === preset);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [key] = useState(() => `duplicate-${newUuid()}`);
  const changed = Object.fromEntries(
    Object.entries(overrides).filter(([id, value]) =>
      coreOptions.find((option) => option.id === id)?.currentValue !== value
    ),
  );
  const waitForTurn = mode === "fork" && fork === "busy";
  const run = async () => {
    setBusy(true);
    setError("");
    try {
      const receipt = mode === "fork"
        ? await forkSession(session.id, { operation: key, config: changed })
        : await duplicateSession(session.id, {
          provider,
          provider_account_id: subscription.value,
          preset: sameAgent ? null : preset || null,
          config: sameAgent ? changed : {},
        }, key);
      onDone();
      closeSessionSettings(session.id);
      requestPickSession(receipt.session_id, receipt.session ?? undefined);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Duplicate failed");
    } finally {
      setBusy(false);
    }
  };
  return (
    <Stack spacing={1.5} sx={{ pt: 1 }} data-duplicate-editor>
      <AiInstallationPicker
        installations={placement.installations}
        value={placement.installation?.value || ""}
        onChange={(value) => {
          setPreset("");
          setOverrides({});
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
      {sameAgent
        ? coreOptions.map((option) => (
          <TextField
            key={option.id}
            select
            size="small"
            label={option.name}
            data-duplicate-option={option.id}
            value={String(overrides[option.id] ?? option.currentValue)}
            onChange={(event) => {
              const choice = option.options.find((entry) =>
                String(entry.value) === event.target.value
              );
              if (choice) setOverrides((previous) => ({ ...previous, [option.id]: choice.value }));
            }}
          >
            {option.options.map((entry) => (
              <MenuItem key={String(entry.value)} value={String(entry.value)}>
                {entry.name}
              </MenuItem>
            ))}
          </TextField>
        ))
        : (
          <SessionPresetPicker
            provider={provider}
            presets={presets}
            value={preset}
            onChange={setPreset}
            defaultLabel="Inherit compatible settings"
            helperText={validPreset ? "" : "Choose an available preset for this agent"}
          />
        )}
      <Typography variant="caption" color="text.secondary" data-duplicate-mode={mode}>
        {mode === "fork"
          ? "Copies the whole conversation into a new session (native fork). This session is unchanged."
          : "Starts a new agent that reads this conversation as linked context on its first message. This session is unchanged."}
      </Typography>
      {error && <Alert severity="error">{error}</Alert>}
      <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap" alignItems="center">
        <Button
          variant="contained"
          data-duplicate-run
          disabled={busy || waitForTurn || !placement.ready || !subscription.allowed ||
            !validPreset}
          onClick={() => void run()}
        >
          {busy ? "Duplicating…" : "Duplicate"}
        </Button>
        <Button disabled={busy} onClick={onDone}>Cancel</Button>
        {waitForTurn && (
          <Typography variant="caption" color="text.secondary">
            Available after the current turn
          </Typography>
        )}
      </Stack>
    </Stack>
  );
}
