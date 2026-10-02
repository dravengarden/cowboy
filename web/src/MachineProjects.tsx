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
import type { MachineSummary } from "./protocol";
import { ConfirmSheet } from "./Sheet";
import {
  defaultProjectPolicy,
  type MachineProjectPolicy,
  projectJson,
  type ProjectPolicies,
} from "./projectPlacement";

interface Project {
  id: string;
  display_name: string;
  canonical_path: string;
}
interface Registry {
  schema: number;
  revision: string;
  managed: boolean;
  projects: Project[];
}
interface Discovery {
  paths: string[];
  truncated: boolean;
}

export function MachineProjects(
  { machine, machines }: {
    machine: MachineSummary;
    machines: readonly MachineSummary[];
  },
): React.JSX.Element {
  const base = `/api/machines/${encodeURIComponent(machine.id)}`;
  const [registry, setRegistry] = useState<Registry>();
  const [policies, setPolicies] = useState<ProjectPolicies>();
  const [policy, setPolicy] = useState<MachineProjectPolicy>(
    defaultProjectPolicy,
  );
  const [preferred, setPreferred] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [path, setPath] = useState("");
  const [name, setName] = useState("");
  const [editing, setEditing] = useState("");
  const [root, setRoot] = useState("");
  const [discovery, setDiscovery] = useState<Discovery>();
  const [removing, setRemoving] = useState<Project>();
  useEffect(() => {
    const controller = new AbortController();
    const options = { signal: controller.signal };
    const fail = (error: unknown): void => {
      if (!controller.signal.aborted) setError(String(error));
    };
    void projectJson<Registry>(`${base}/projects`, options).then((value) => {
      if (!controller.signal.aborted) setRegistry(value);
    }).catch(fail);
    void projectJson<ProjectPolicies>("/api/project-policies", options).then(
      (value) => {
        if (controller.signal.aborted) return;
        setPolicies(value);
        setPolicy(value.machines[machine.id] ?? defaultProjectPolicy);
        setPreferred(value.default_runtime_machine_id === machine.id);
      },
    ).catch(fail);
    return (): void => controller.abort();
  }, [base, machine.id]);
  const run = async (action: () => Promise<void>): Promise<void> => {
    if (busy) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await action();
    } catch (error) {
      setError(String(error));
      // Observe a lost response or CAS conflict; never replay a mutation.
      const observed = await Promise.allSettled([
        projectJson<Registry>(`${base}/projects`),
        projectJson<ProjectPolicies>("/api/project-policies"),
      ]);
      if (observed[0].status === "fulfilled") setRegistry(observed[0].value);
      if (observed[1].status === "fulfilled") {
        const current = observed[1].value;
        setPolicies(current);
        setPolicy(current.machines[machine.id] ?? defaultProjectPolicy);
        setPreferred(current.default_runtime_machine_id === machine.id);
      }
    } finally {
      setBusy(false);
    }
  };
  const request = <T,>(
    suffix: string,
    body: unknown,
    method = "POST",
  ): Promise<T> =>
    projectJson<T>(`${base}/${suffix}`, {
      method,
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
  const register = (): void => {
    void run(async () => {
      const value = await request<Registry>("projects", {
        action: "upsert",
        expected_revision: registry?.revision,
        project: {
          id: editing || `project-${crypto.randomUUID()}`,
          display_name: name.trim(),
          canonical_path: path.trim(),
        },
      });
      setRegistry(value);
      setEditing("");
      setName("");
      setPath("");
      setNotice("Project saved.");
    });
  };
  return (
    <Stack spacing={1.5}>
      <Typography variant="overline" color="text.secondary">
        AI and projects
      </Typography>
      {error && <Alert severity="error">{error}</Alert>}
      {notice && <Alert severity="success">{notice}</Alert>}
      {policies && (
        <>
          <TextField
            select
            size="small"
            label="AI mode"
            value={policy.agent_mode}
            disabled={busy}
            onChange={(event) =>
              setPolicy({
                ...policy,
                agent_mode: event.target
                  .value as MachineProjectPolicy["agent_mode"],
              })}
          >
            <MenuItem value="disabled">No AI runtime</MenuItem>
            <MenuItem value="local">Local projects only</MenuItem>
            <MenuItem value="remote">Remote projects only</MenuItem>
            <MenuItem value="either">Local and remote projects</MenuItem>
          </TextField>
          <FormControlLabel
            control={
              <Checkbox
                checked={policy.hosts_projects}
                disabled={busy}
                onChange={(_, checked) =>
                  setPolicy({ ...policy, hosts_projects: checked })}
              />
            }
            label="Offer projects on this Machine"
          />
          {policy.agent_mode !== "disabled" && (
            <FormControlLabel
              control={
                <Checkbox
                  checked={preferred}
                  disabled={busy}
                  onChange={(_, checked) => setPreferred(checked)}
                />
              }
              label="Prefer this Machine for AI"
            />
          )}
          {(policy.agent_mode === "remote" || policy.agent_mode === "either") &&
            (
              <>
                <FormControlLabel
                  control={
                    <Checkbox
                      checked={policy.remote_targets === null}
                      disabled={busy}
                      onChange={(_, checked) =>
                        setPolicy({
                          ...policy,
                          remote_targets: checked ? null : [],
                        })}
                    />
                  }
                  label="Allow all enrolled project Machines"
                />
                {policy.remote_targets !== null && (
                  <TextField
                    select
                    size="small"
                    label="Allowed project Machines"
                    value={policy.remote_targets}
                    disabled={busy}
                    SelectProps={{ multiple: true }}
                    onChange={(event) =>
                      setPolicy({
                        ...policy,
                        remote_targets: typeof event.target.value === "string"
                          ? event.target.value.split(",")
                          : event.target.value,
                      })}
                  >
                    {machines.filter((m) => m.id !== machine.id).map((m) => (
                      <MenuItem value={m.id} key={m.id}>
                        {m.display_name}
                      </MenuItem>
                    ))}
                  </TextField>
                )}
              </>
            )}
          <Button
            variant="outlined"
            disabled={busy}
            onClick={() => {
              void run(async () => {
                const value = await request<ProjectPolicies>("project-policy", {
                  expected_revision: policies.revision,
                  policy,
                  preferred: policy.agent_mode !== "disabled" && preferred,
                }, "PUT");
                setPolicies(value);
                setNotice(
                  "Machine policy saved. Existing sessions keep their placement.",
                );
              });
            }}
          >
            Save Machine policy
          </Button>
        </>
      )}
      <Typography variant="overline" color="text.secondary">
        Projects on {machine.display_name}
      </Typography>
      <Typography variant="caption" color="text.secondary">
        Git projects use session worktrees. Other directories are shared in
        place. Names can contain / for grouping.
      </Typography>
      {(registry?.projects ?? machine.workspaces).map((project) => (
        <Stack
          key={project.id}
          direction="row"
          alignItems="center"
          spacing={0.5}
        >
          <Box sx={{ flex: 1, minWidth: 0 }}>
            <Typography variant="body2">{project.display_name}</Typography>
            <Typography
              variant="caption"
              color="text.secondary"
              sx={{ overflowWrap: "anywhere" }}
            >
              {project.canonical_path}
            </Typography>
          </Box>
          <Button
            size="small"
            disabled={busy || !registry}
            onClick={() => {
              setEditing(project.id);
              setName(project.display_name);
              setPath(project.canonical_path);
            }}
          >
            Rename
          </Button>
          <Button
            size="small"
            disabled={busy || !registry}
            onClick={() => setRemoving(project)}
          >
            Remove
          </Button>
        </Stack>
      ))}
      {registry && (
        <>
          <TextField
            size="small"
            label="Project directory"
            value={path}
            disabled={busy || Boolean(editing)}
            onChange={(event) => setPath(event.target.value)}
            placeholder="/home/user/projects/example"
          />
          <TextField
            size="small"
            label="Project name"
            value={name}
            disabled={busy}
            onChange={(event) => setName(event.target.value)}
            placeholder="columbus/cowboy"
          />
          <Stack direction="row" spacing={1}>
            <Button
              variant="outlined"
              disabled={busy || !path.trim() || !name.trim()}
              onClick={register}
            >
              {editing ? "Save name" : "Register project"}
            </Button>
            {editing && (
              <Button
                disabled={busy}
                onClick={() => {
                  setEditing("");
                  setName("");
                  setPath("");
                }}
              >
                Cancel rename
              </Button>
            )}
          </Stack>
          <TextField
            size="small"
            label="Discover in directory"
            value={root}
            disabled={busy}
            onChange={(event) => setRoot(event.target.value)}
            placeholder="/home/user/projects"
          />
          <Button
            disabled={busy || !root.trim()}
            onClick={() => {
              void run(async () =>
                setDiscovery(
                  await request<Discovery>("projects", {
                    action: "discover",
                    root: root.trim(),
                  }),
                )
              );
            }}
          >
            Find projects
          </Button>
          {discovery && (
            <>
              {discovery.truncated && (
                <Typography variant="caption">
                  Search limited to nearby directories. Choose a narrower root
                  for more results.
                </Typography>
              )}
              {discovery.paths.map((candidate) => (
                <Button
                  key={candidate}
                  size="small"
                  sx={{
                    justifyContent: "flex-start",
                    textTransform: "none",
                    overflowWrap: "anywhere",
                  }}
                  disabled={registry.projects.some((p) =>
                    p.canonical_path === candidate
                  )}
                  onClick={() => {
                    setEditing("");
                    setPath(candidate);
                    setName(
                      candidate.split("/").filter(Boolean).at(-1) ?? "Project",
                    );
                  }}
                >
                  {candidate}
                </Button>
              ))}
            </>
          )}
        </>
      )}
      <ConfirmSheet
        open={Boolean(removing)}
        onClose={() => !busy && setRemoving(undefined)}
        title="Remove project from Cowboy?"
        actions={
          <>
            <Button disabled={busy} onClick={() => setRemoving(undefined)}>
              Cancel
            </Button>
            <Button
              color="error"
              disabled={busy}
              onClick={() => {
                void run(async () => {
                  setRegistry(
                    await request<Registry>("projects", {
                      action: "remove",
                      expected_revision: registry?.revision,
                      id: removing?.id,
                    }),
                  );
                  setRemoving(undefined);
                });
              }}
            >
              Remove project
            </Button>
          </>
        }
      >
        <Typography>
          {removing?.display_name}{" "}
          will leave the New session picker. Files and existing sessions are
          kept.
        </Typography>
      </ConfirmSheet>
    </Stack>
  );
}
