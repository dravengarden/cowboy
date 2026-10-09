import React, { useState } from "react";
import {
  Box,
  Button,
  Checkbox,
  MenuItem,
  Select,
  Stack,
  Switch,
  Typography,
} from "@mui/material";
import {
  agentLabel,
  type AgentTools,
  AUTO,
  CONCURRENCY_CHOICES,
  fetchAgentTools,
  fetchSessionTools,
  normalizedDefault,
  overridden,
  overrideFor,
  saveAgentTools,
  saveSessionTools,
  SESSION_LIMIT_CHOICES,
  type ToolsCatalog,
  useRemoteDocument,
  withTarget,
} from "./agentTools";

function Row(
  { label, description, children }: {
    label: string;
    description?: string;
    children: React.ReactNode;
  },
): React.JSX.Element {
  return (
    <Stack
      direction="row"
      alignItems="center"
      justifyContent="space-between"
      spacing={2}
      sx={{ py: 0.75, minHeight: 44 }}
    >
      <Stack sx={{ minWidth: 0 }}>
        <Typography variant="body2">{label}</Typography>
        {description && (
          <Typography variant="caption" color="text.secondary">
            {description}
          </Typography>
        )}
      </Stack>
      <Box
        sx={{ flexShrink: 0, display: "flex", alignItems: "center", gap: 1 }}
      >
        {children}
      </Box>
    </Stack>
  );
}

function Heading(
  { children }: { children: React.ReactNode },
): React.JSX.Element {
  return (
    <Typography
      variant="overline"
      color="text.secondary"
      sx={{ letterSpacing: 0.8, lineHeight: 1.6, display: "block", pt: 1 }}
    >
      {children}
    </Typography>
  );
}

function choices(values: readonly number[], current: number): number[] {
  return [...new Set([...values, current])].sort((left, right) => left - right);
}

/** One editor for a session's effective tools and an agent kind's defaults. */
export function AgentToolsEditor(
  { value, catalog, disabled, onChange }: {
    value: AgentTools;
    catalog: ToolsCatalog;
    disabled: boolean;
    onChange: (next: AgentTools) => void;
  },
): React.JSX.Element {
  const calls = value.calls;
  const setCalls = (next: Partial<AgentTools["calls"]>): void => {
    const merged = { ...calls, ...next };
    onChange({
      ...value,
      calls: { ...merged, default: normalizedDefault(merged) },
    });
  };
  const allowed = calls.targets.map((target) => target.agent);
  return (
    <Box data-agent-tools-editor>
      <Heading>Agent calls</Heading>
      <Row
        label="Allow calls"
        description="Start read-only Codex or Claude reviewers from this agent. Off costs no tokens."
      >
        <Switch
          checked={calls.enabled}
          disabled={disabled}
          onChange={(_, checked): void => setCalls({ enabled: checked })}
          slotProps={{ input: { "aria-label": "Allow agent calls" } }}
        />
      </Row>
      <Row
        label="Default agent"
        description="Auto prefers a different agent than the caller"
      >
        <Select
          size="small"
          value={normalizedDefault(calls)}
          disabled={disabled || !calls.enabled}
          onChange={(event): void =>
            setCalls({ default: String(event.target.value) })}
          inputProps={{ "aria-label": "Default called agent" }}
          sx={{ minWidth: 128 }}
        >
          <MenuItem value={AUTO}>Auto</MenuItem>
          {allowed.map((agent) => (
            <MenuItem key={agent} value={agent}>{agentLabel(agent)}</MenuItem>
          ))}
        </Select>
      </Row>
      {catalog.call_targets.map(({ agent, presets }) => {
        const target = calls.targets.find((candidate) =>
          candidate.agent === agent
        );
        return (
          <Row
            key={agent}
            label={agentLabel(agent)}
            description="Model and reasoning for calls to this agent"
          >
            <Select
              size="small"
              value={target?.preset ?? ""}
              displayEmpty
              disabled={disabled || !calls.enabled || target === undefined}
              onChange={(event): void => {
                const preset = String(event.target.value);
                setCalls({
                  targets: withTarget(
                    calls.targets,
                    agent,
                    true,
                    preset || undefined,
                  ),
                });
              }}
              inputProps={{
                "aria-label": `${agentLabel(agent)} model and reasoning`,
              }}
              sx={{ minWidth: 168, maxWidth: 220 }}
            >
              <MenuItem value="">Agent default</MenuItem>
              {presets.map((preset) => (
                <MenuItem key={preset.id} value={preset.id}>
                  {preset.name}
                </MenuItem>
              ))}
            </Select>
            <Checkbox
              checked={target !== undefined}
              disabled={disabled || !calls.enabled}
              onChange={(_, checked): void =>
                setCalls({
                  targets: withTarget(
                    calls.targets,
                    agent,
                    checked,
                    target?.preset,
                  ),
                })}
              inputProps={{
                "aria-label": `Allow calls to ${agentLabel(agent)}`,
              }}
            />
          </Row>
        );
      })}
      <Row
        label="Concurrent calls"
        description="Running at once from one session"
      >
        <Select
          size="small"
          value={calls.max_concurrent}
          disabled={disabled || !calls.enabled}
          onChange={(event): void =>
            setCalls({ max_concurrent: Number(event.target.value) })}
          inputProps={{ "aria-label": "Concurrent call limit" }}
        >
          {choices(CONCURRENCY_CHOICES, calls.max_concurrent).map((limit) => (
            <MenuItem key={limit} value={limit}>{limit}</MenuItem>
          ))}
        </Select>
      </Row>
      <Row
        label="Calls per session"
        description="Total budget before calls are refused"
      >
        <Select
          size="small"
          value={calls.max_per_session}
          disabled={disabled || !calls.enabled}
          onChange={(event): void =>
            setCalls({ max_per_session: Number(event.target.value) })}
          inputProps={{ "aria-label": "Calls per session" }}
        >
          {choices(SESSION_LIMIT_CHOICES, calls.max_per_session).map((
            limit,
          ) => <MenuItem key={limit} value={limit}>{limit}</MenuItem>)}
        </Select>
      </Row>
      <Heading>Matrix memory</Heading>
      <Row label="Memory tools" description="Search and record shared memory">
        <Switch
          checked={value.matrix.tools}
          disabled={disabled}
          onChange={(_, checked): void =>
            onChange({ ...value, matrix: { ...value.matrix, tools: checked } })}
          slotProps={{ input: { "aria-label": "Matrix memory tools" } }}
        />
      </Row>
      <Row
        label="Automatic recall"
        description="Relevant memory before each turn"
      >
        <Switch
          checked={value.matrix.recall}
          disabled={disabled}
          onChange={(_, checked): void =>
            onChange({
              ...value,
              matrix: { ...value.matrix, recall: checked },
            })}
          slotProps={{ input: { "aria-label": "Matrix automatic recall" } }}
        />
      </Row>
      <Typography
        variant="caption"
        color="text.secondary"
        sx={{ display: "block", pb: 1 }}
      >
        Matrix switches are saved now and take effect when the agent's Provider
        supports them; until then the host's Matrix enrollment applies.
      </Typography>
    </Box>
  );
}

/** A session's tools: its agent kind's defaults with this session's changes. */
export function SessionToolsSection(
  { sessionId }: { sessionId: string },
): React.JSX.Element {
  const { value, error, saving, save } = useRemoteDocument(
    () => fetchSessionTools(sessionId),
    sessionId,
  );
  return (
    <Box data-session-tools sx={{ py: 1.5 }}>
      <Stack direction="row" alignItems="center" justifyContent="space-between">
        <Typography
          variant="overline"
          color="text.secondary"
          sx={{ letterSpacing: 0.8 }}
        >
          Tools
        </Typography>
        {value && (
          overridden(value.override)
            ? (
              <Button
                size="small"
                disabled={saving}
                onClick={(): void =>
                  save(() => saveSessionTools(sessionId, { schema: 1 }))}
              >
                Use {agentLabel(value.agent)} defaults
              </Button>
            )
            : (
              <Typography variant="caption" color="text.secondary">
                {agentLabel(value.agent)} defaults
              </Typography>
            )
        )}
      </Stack>
      {error && <Typography variant="caption" color="error">{error}
      </Typography>}
      {value && (
        <AgentToolsEditor
          value={value.effective}
          catalog={value.catalog}
          disabled={saving}
          onChange={(next): void => {
            const override = overrideFor(value.defaults, next);
            save(
              () => saveSessionTools(sessionId, override),
              { ...value, effective: next, override },
            );
          }}
        />
      )}
    </Box>
  );
}

/** Defaults per agent kind; every session of that kind inherits them. */
export function AgentToolsSettings(): React.JSX.Element {
  const { value, error, saving, save } = useRemoteDocument(
    fetchAgentTools,
    "agent-tools",
  );
  const [selected, setSelected] = useState<string | null>(null);
  const agents = value?.agents ?? [];
  const current = agents.find((entry) => entry.agent === selected) ?? agents[0];
  return (
    <Box data-agent-tools-settings sx={{ px: 1.5, pb: 1 }}>
      <Typography
        variant="caption"
        color="text.secondary"
        sx={{ display: "block", pt: 1 }}
      >
        Defaults for every session of an agent kind. A session can change them
        in its Tools.
      </Typography>
      {error && <Typography variant="caption" color="error">{error}
      </Typography>}
      {current && value && (
        <>
          <Row label="Agent">
            <Select
              size="small"
              value={current.agent}
              onChange={(event): void =>
                setSelected(String(event.target.value))}
              inputProps={{ "aria-label": "Agent kind" }}
              sx={{ minWidth: 168 }}
            >
              {agents.map((entry) => (
                <MenuItem key={entry.agent} value={entry.agent}>
                  {agentLabel(entry.agent)}
                </MenuItem>
              ))}
            </Select>
            {current.customized && (
              <Button
                size="small"
                disabled={saving}
                onClick={(): void =>
                  save(async () => {
                    const saved = await saveAgentTools(current.agent, null);
                    return {
                      ...value,
                      agents: agents.map((entry) =>
                        entry.agent === saved.agent ? saved : entry
                      ),
                    };
                  })}
              >
                Reset
              </Button>
            )}
          </Row>
          <AgentToolsEditor
            value={current.settings}
            catalog={value.catalog}
            disabled={saving}
            onChange={(next): void => {
              const optimistic = {
                ...value,
                agents: agents.map((entry) =>
                  entry.agent === current.agent
                    ? { ...entry, settings: next, customized: true }
                    : entry
                ),
              };
              save(async () => {
                const saved = await saveAgentTools(current.agent, next);
                return {
                  ...value,
                  agents: agents.map((entry) =>
                    entry.agent === saved.agent ? saved : entry
                  ),
                };
              }, optimistic);
            }}
          />
        </>
      )}
    </Box>
  );
}
