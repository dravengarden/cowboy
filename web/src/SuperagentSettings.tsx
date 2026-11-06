import { useEffect, useRef, useState } from "react";
import {
  Alert,
  Box,
  Button,
  FormControlLabel,
  MenuItem,
  Stack,
  Switch,
  TextField,
  Typography,
} from "@mui/material";
import { useStoreSelector } from "./store";
import { productSessionSignal } from "./productSessionEnd";
import {
  parsePatternBundle,
  sourceProfileId,
  type SuperagentConfig,
  type SuperagentMode,
  type SuperagentProject,
  useSuperagentSettings,
} from "./superagentPreferences";

const modes = [
  ["off", "Off"],
  ["observe", "Observe"],
  ["coordinate", "Coordinate"],
] as const;

export function SessionSuperagentSettings({ sessionId }: { sessionId: string }): React.JSX.Element {
  const settings = useSuperagentSettings();
  const config = settings.config;
  const participation = config?.sessions[sessionId];
  const [draftMandate, setDraftMandate] = useState<string>();
  useEffect(() => {
    setDraftMandate(undefined);
  }, [sessionId, participation?.project, participation?.mandate]);
  if (!config) {
    return settings.error ? (
      <Alert severity="error" action={<Button onClick={() => void settings.retry()}>Retry</Button>}>
        {settings.error}
      </Alert>
    ) : (
      <Typography variant="body2">{settings.error ?? "Loading Super-agent…"}</Typography>
    );
  }
  const coordinator = Object.entries(config.projects).find(
    ([, project]) => project.coordinator === sessionId,
  );
  if (coordinator) {
    return (
      <Alert severity="info">
        This session coordinates {coordinator[0]}. Manage its scope in Settings → Agent tools →
        Super-agent.
      </Alert>
    );
  }
  const update = (project: string, mode: SuperagentMode | null, mandate: string) => {
    const sessions = { ...config.sessions };
    if (project) sessions[sessionId] = { project, mode, mandate };
    else delete sessions[sessionId];
    return settings.save({ ...config, sessions });
  };
  const busy = settings.saving || !!settings.pending;
  return (
    <Stack spacing={1.5}>
      <Typography variant="subtitle2">Super-agent</Typography>
      <Typography variant="caption" color="text.secondary">
        Choose whether this session participates. Coordination follows the task’s existing
        permissions.
      </Typography>
      <TextField
        select
        size="small"
        label="Project"
        slotProps={{
          select: { displayEmpty: true },
          inputLabel: { shrink: true },
        }}
        value={participation?.project ?? ""}
        disabled={busy}
        onChange={(event) => {
          setDraftMandate(undefined);
          void update(event.target.value, "off", participation?.mandate ?? "");
        }}
      >
        <MenuItem value="">Not enrolled</MenuItem>
        {Object.keys(config.projects).map((id) => (
          <MenuItem key={id} value={id}>
            {id}
          </MenuItem>
        ))}
      </TextField>
      {participation && (
        <>
          <TextField
            multiline
            minRows={2}
            size="small"
            label="Task scope"
            value={draftMandate ?? participation.mandate}
            helperText="Describe the work already requested. Adding a scope does not authorize a push or deployment."
            onChange={(event) => setDraftMandate(event.target.value)}
            slotProps={{ htmlInput: { maxLength: 2000 } }}
          />
          {draftMandate !== undefined && draftMandate !== participation.mandate && (
            <Button
              disabled={busy}
              onClick={() => {
                void update(participation.project, participation.mode, draftMandate);
              }}
            >
              Save task scope
            </Button>
          )}
          <TextField
            select
            size="small"
            label="Participation"
            value={participation.mode ?? "inherit"}
            disabled={busy}
            onChange={(event) => {
              void update(
                participation.project,
                event.target.value === "inherit" ? null : (event.target.value as SuperagentMode),
                draftMandate ?? participation.mandate,
              );
            }}
          >
            <MenuItem value="inherit">
              Inherit project ({config.projects[participation.project]?.mode ?? "off"})
            </MenuItem>
            {modes.map(([value, label]) => (
              <MenuItem key={value} value={value}>
                {label}
              </MenuItem>
            ))}
          </TextField>
          <Typography variant="caption" color="text.secondary">
            Observe assesses results. Coordinate can send bounded follow-ups to this session. Pause
            and Stop hold automatic work.
          </Typography>
        </>
      )}
      {settings.error && (
        <Alert
          severity="error"
          action={<Button onClick={() => void settings.retry()}>Retry</Button>}
        >
          {settings.error}
        </Alert>
      )}
    </Stack>
  );
}

export function SuperagentSettings(): React.JSX.Element {
  const settings = useSuperagentSettings();
  const sessions = useStoreSelector((state) => state.sessions);
  const [importError, setImportError] = useState<string>();
  const picker = useRef<HTMLInputElement>(null);
  const config = settings.config;
  if (!config) {
    return settings.error ? (
      <Alert severity="error" action={<Button onClick={() => void settings.retry()}>Retry</Button>}>
        {settings.error}
      </Alert>
    ) : (
      <Typography variant="body2">{settings.error ?? "Loading Super-agent…"}</Typography>
    );
  }
  const busy = settings.saving || !!settings.pending;
  const profiles = config.sources.map(sourceProfileId).filter((id): id is string => !!id);
  const change = (id: string, value: SuperagentProject) =>
    settings.save({ ...config, projects: { ...config.projects, [id]: value } });
  const importSources = async (file: File) => {
    try {
      if (file.size > 800_000) throw new Error("Pattern bundle is too large.");
      const sources = parsePatternBundle(await file.text());
      setImportError(undefined);
      await settings.save({ ...config, sources });
    } catch (cause) {
      setImportError(String(cause));
    }
  };
  return (
    <Stack spacing={2} sx={{ py: 1 }}>
      <Typography variant="subtitle1">Super-agent</Typography>
      <Typography variant="body2" color="text.secondary">
        A dedicated Astra Max or Fable Max session supervises participating sessions. No session
        joins automatically. Keep the coordinator’s native workspace access appropriate for the
        project.
      </Typography>
      <Box>
        <Button disabled={busy} onClick={() => picker.current?.click()}>
          Import pattern bundle
        </Button>
        <input
          ref={picker}
          type="file"
          accept="application/json,.json"
          hidden
          onChange={(event) => {
            const file = event.target.files?.[0];
            event.target.value = "";
            if (file) void importSources(file);
          }}
        />
        <Typography variant="caption" display="block" color="text.secondary">
          Global → workspace → project. {profiles.length} profiles installed. Import replaces the
          library and validates all bindings before saving.
        </Typography>
      </Box>
      {Object.entries(config.projects).map(([id, project]) => (
        <ProjectPolicy
          key={id}
          id={id}
          project={project}
          busy={busy}
          change={change}
          remove={() => {
            const projects = { ...config.projects };
            delete projects[id];
            void settings.save({
              ...config,
              projects,
              sessions: Object.fromEntries(
                Object.entries(config.sessions).filter(([, value]) => value.project !== id),
              ),
            });
          }}
          title={sessions.find((s) => s.id === project.coordinator)?.title ?? project.coordinator}
        />
      ))}
      <AddProject config={config} profiles={profiles} busy={busy} change={change} />
      {(settings.error || importError) && (
        <Alert
          severity="error"
          action={<Button onClick={() => void settings.retry()}>Retry</Button>}
        >
          {settings.error ?? importError}
        </Alert>
      )}
      <SuperagentActivity config={config} />
    </Stack>
  );
}

function AddProject({
  config,
  profiles,
  busy,
  change,
}: {
  config: SuperagentConfig;
  profiles: string[];
  busy: boolean;
  change: (id: string, value: SuperagentProject) => Promise<void>;
}): React.JSX.Element {
  const sessions = useStoreSelector((state) => state.sessions);
  const [id, setId] = useState("");
  const [draft, setDraft] = useState<SuperagentProject>({
    profile: "",
    coordinator: "",
    preset: "astra-max",
    mode: "observe",
    max_followups: 2,
    learning: false,
  });
  return (
    <>
      <Typography variant="subtitle2">Add project</Typography>
      <TextField
        size="small"
        label="Project ID"
        value={id}
        onChange={(event) => setId(event.target.value)}
      />
      <TextField
        select
        size="small"
        label="Pattern profile"
        value={draft.profile}
        onChange={(event) => setDraft({ ...draft, profile: event.target.value })}
      >
        {profiles.map((id) => (
          <MenuItem key={id} value={id}>
            {id}
          </MenuItem>
        ))}
      </TextField>
      <TextField
        select
        size="small"
        label="Model"
        value={draft.preset}
        onChange={(event) =>
          setDraft({
            ...draft,
            preset: event.target.value as SuperagentProject["preset"],
          })
        }
      >
        <MenuItem value="astra-max">Astra Max</MenuItem>
        <MenuItem value="fable-max">Fable Max</MenuItem>
      </TextField>
      <TextField
        select
        size="small"
        label="Coordinator session"
        value={draft.coordinator}
        helperText="Create an idle dedicated session with the matching Max preset, on the same execution Machine as its workers."
        onChange={(event) => setDraft({ ...draft, coordinator: event.target.value })}
      >
        {sessions
          .filter(
            (session) =>
              !session.system &&
              !session.trashed_at_ms &&
              session.provider === (draft.preset === "astra-max" ? "codex" : "claude-code"),
          )
          .map((session) => (
            <MenuItem key={session.id} value={session.id}>
              {session.title || session.id}
            </MenuItem>
          ))}
      </TextField>
      <Button
        disabled={busy || !id || !draft.profile || !draft.coordinator || !!config.projects[id]}
        onClick={() => void change(id, draft)}
      >
        Add in Observe mode
      </Button>
    </>
  );
}

function ProjectPolicy({
  id,
  project,
  title,
  busy,
  change,
  remove,
}: {
  id: string;
  project: SuperagentProject;
  title: string;
  busy: boolean;
  change: (id: string, value: SuperagentProject) => Promise<void>;
  remove: () => void;
}): React.JSX.Element {
  return (
    <Stack spacing={1} sx={{ border: 1, borderColor: "divider", borderRadius: 1, p: 1.5 }}>
      <Typography variant="subtitle2">{id}</Typography>
      <Typography variant="caption">
        {project.profile} · {project.preset} · {title}
      </Typography>
      <TextField
        select
        size="small"
        label="Default participation"
        value={project.mode}
        disabled={busy}
        onChange={(event) =>
          void change(id, {
            ...project,
            mode: event.target.value as SuperagentMode,
          })
        }
      >
        {modes.map(([value, label]) => (
          <MenuItem key={value} value={value}>
            {label}
          </MenuItem>
        ))}
      </TextField>
      <TextField
        select
        size="small"
        label="Follow-ups per human task"
        value={project.max_followups}
        disabled={busy}
        onChange={(event) =>
          void change(id, {
            ...project,
            max_followups: Number(event.target.value),
          })
        }
      >
        {[1, 2, 3, 4, 5].map((limit) => (
          <MenuItem key={limit} value={limit}>
            {limit}
          </MenuItem>
        ))}
      </TextField>
      <FormControlLabel
        label="Propose pattern improvements"
        control={
          <Switch
            disabled={busy}
            checked={project.learning}
            onChange={(_, learning) => void change(id, { ...project, learning })}
          />
        }
      />
      <Button disabled={busy} onClick={remove}>
        Remove project supervision
      </Button>
      <Typography variant="caption" color="text.secondary">
        Removing supervision unenrolls its workers and releases the coordinator role. Sessions are
        kept.
      </Typography>
    </Stack>
  );
}

interface Activity {
  operation: string;
  created_at_ms: number;
  result: {
    kind?: string;
    status?: string;
    project?: string;
    session_id?: string;
    markdown?: string;
  } | null;
}
function SuperagentActivity({ config }: { config: SuperagentConfig }): React.JSX.Element {
  const [activity, setActivity] = useState<Activity[]>([]);
  const [error, setError] = useState<string>();
  const load = async () => {
    try {
      const response = await fetch("/api/superagent/activity", {
        cache: "no-store",
        signal: productSessionSignal(),
      });
      if (!response.ok) throw new Error("Could not load supervision activity.");
      setActivity((await response.json()).activity);
      setError(undefined);
    } catch (cause) {
      setError(String(cause));
    }
  };
  return (
    <Stack spacing={1}>
      <Button onClick={() => void load()}>Load recent activity and pattern candidates</Button>
      {error && <Alert severity="error">{error}</Alert>}
      {activity
        .filter(
          (item) =>
            !item.operation.startsWith("sa-request-") &&
            (!item.result?.project || config.projects[item.result.project]),
        )
        .map((item) => (
          <Box key={item.operation} sx={{ borderTop: 1, borderColor: "divider", pt: 1 }}>
            <Typography variant="body2">
              {item.result?.project} · {item.result?.kind ?? "Reserved operation"} ·{" "}
              {item.result?.status ?? "Outcome unknown"}
            </Typography>
            <Typography variant="caption" color="text.secondary">
              {new Date(item.created_at_ms).toLocaleString()} · {item.result?.session_id}
            </Typography>
            {item.result?.markdown && (
              <>
                <Typography variant="caption" display="block">
                  Candidate only. Review applicability, evidence and counterexamples before adding a
                  new version to the source library.
                </Typography>
                <Button
                  onClick={() => {
                    const url = URL.createObjectURL(
                      new Blob([item.result?.markdown ?? ""], {
                        type: "text/markdown",
                      }),
                    );
                    const link = document.createElement("a");
                    link.href = url;
                    link.download = "pattern-candidate.md";
                    link.click();
                    URL.revokeObjectURL(url);
                  }}
                >
                  Download Markdown candidate
                </Button>
              </>
            )}
          </Box>
        ))}
    </Stack>
  );
}
