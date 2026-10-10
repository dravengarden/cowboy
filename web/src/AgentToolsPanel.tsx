import React, { useState } from "react";
import Check from "@mui/icons-material/Check";
import {
  alpha,
  Box,
  Button,
  ButtonBase,
  Chip,
  Collapse,
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
  callableTargets,
  sameFamily,
  type ToolsCatalog,
  useRemoteDocument,
  withTarget,
} from "./agentTools";
import { ProviderIcon } from "./ProviderIcon";

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

type Preset = ToolsCatalog["call_targets"][number]["presets"][number];

/** One selectable model-and-reasoning choice inside a target card, in the
 * same material as the Run configuration preset cards. */
function PresetChoice(
  { name, detail, tag, selected, disabled, onSelect }: {
    name: string;
    detail: string;
    tag?: string | undefined;
    selected: boolean;
    disabled: boolean;
    onSelect: () => void;
  },
): React.JSX.Element {
  return (
    <ButtonBase
      role="radio"
      aria-checked={selected}
      aria-label={name}
      disabled={disabled}
      onClick={onSelect}
      sx={{
        width: "100%",
        minHeight: 48,
        px: 1.25,
        py: 0.75,
        borderRadius: 1,
        border: 1,
        borderColor: selected ? "primary.main" : "divider",
        bgcolor: (theme) =>
          selected
            ? alpha(theme.palette.primary.main, 0.13)
            : alpha(theme.palette.background.default, 0.34),
        textAlign: "left",
        justifyContent: "flex-start",
        touchAction: "manipulation",
        "&.Mui-disabled": { opacity: 0.46 },
      }}
    >
      <Box sx={{ flex: 1, minWidth: 0 }}>
        <Stack direction="row" spacing={0.75} alignItems="center">
          <Typography variant="body2" sx={{ fontWeight: 650 }}>
            {name}
          </Typography>
          {tag && (
            <Chip
              label={tag}
              size="small"
              color="primary"
              variant="outlined"
              sx={{ height: 20, fontSize: "0.625rem" }}
            />
          )}
        </Stack>
        {detail && (
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ display: "block", mt: 0.2 }}
          >
            {detail}
          </Typography>
        )}
      </Box>
      <Box
        aria-hidden
        sx={{ width: 18, display: "grid", placeItems: "center", flexShrink: 0 }}
      >
        {selected && <Check sx={{ fontSize: 18 }} />}
      </Box>
    </ButtonBase>
  );
}

/** A callable agent: its switch, and when allowed, the model and reasoning
 * its calls use. One card per Provider, so a new Provider is one more card. */
function CallTargetCard(
  { agent, presets, allowed, preset, disabled, onAllowed, onPreset }: {
    agent: string;
    presets: readonly Preset[];
    allowed: boolean;
    preset: string | undefined;
    disabled: boolean;
    onAllowed: (allowed: boolean) => void;
    onPreset: (preset: string | undefined) => void;
  },
): React.JSX.Element {
  const label = agentLabel(agent);
  const chosen = presets.find((candidate) => candidate.id === preset);
  return (
    <Box
      data-call-target={agent}
      sx={{
        border: 1,
        borderColor: allowed && !disabled ? "primary.main" : "divider",
        borderRadius: 1.5,
        bgcolor: (theme) => alpha(theme.palette.background.default, 0.34),
        opacity: disabled ? 0.6 : 1,
        overflow: "hidden",
      }}
    >
      <Stack
        direction="row"
        alignItems="center"
        spacing={1.25}
        sx={{ px: 1.5, py: 1, minHeight: 56 }}
      >
        <ProviderIcon provider={agent} sx={{ fontSize: "1.375rem" }} />
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography variant="body2" sx={{ fontWeight: 700 }}>
            {label}
          </Typography>
          <Typography
            variant="caption"
            color="text.secondary"
            noWrap
            component="div"
          >
            {allowed ? chosen?.name ?? "Agent default" : "Not called"}
          </Typography>
        </Box>
        <Switch
          checked={allowed}
          disabled={disabled}
          onChange={(_, checked): void => onAllowed(checked)}
          slotProps={{ input: { "aria-label": `Allow calls to ${label}` } }}
        />
      </Stack>
      <Collapse in={allowed} unmountOnExit>
        <Stack
          role="radiogroup"
          aria-label={`${label} model and reasoning`}
          spacing={0.75}
          sx={{ px: 1.5, pb: 1.5 }}
        >
          <PresetChoice
            name="Agent default"
            detail={`${label}'s own default model and reasoning`}
            selected={chosen === undefined}
            disabled={disabled}
            onSelect={(): void => onPreset(undefined)}
          />
          {presets.map((candidate) => (
            <PresetChoice
              key={candidate.id}
              name={candidate.name}
              detail={candidate.detail}
              tag={candidate.is_default ? "Provider default" : undefined}
              selected={candidate.id === preset}
              disabled={disabled}
              onSelect={(): void => onPreset(candidate.id)}
            />
          ))}
        </Stack>
      </Collapse>
    </Box>
  );
}

function choices(values: readonly number[], current: number): number[] {
  return [...new Set([...values, current])].sort((left, right) => left - right);
}

/** One editor for a session's effective tools and an agent kind's defaults. */
export function AgentToolsEditor(
  { caller, value, catalog, disabled, onChange }: {
    /** The agent kind these settings belong to; it never calls itself. */
    caller: string;
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
  const targets = callableTargets(caller, catalog);
  const allowed = calls.targets.map((target) => target.agent)
    .filter((agent) => !sameFamily(agent, caller));
  const names = targets.map((target) => agentLabel(target.agent));
  return (
    <Box data-agent-tools-editor sx={{ color: "text.primary" }}>
      <Heading>Agent calls</Heading>
      <Row
        label="Allow calls"
        description={`Start read-only ${
          names.length > 0 ? names.join(" or ") : "agent"
        } reviewers from this agent. Off costs no tokens.`}
      >
        <Switch
          checked={calls.enabled}
          disabled={disabled}
          onChange={(_, checked): void => setCalls({ enabled: checked })}
          slotProps={{ input: { "aria-label": "Allow agent calls" } }}
        />
      </Row>
      {targets.length > 1 && (
        <Row
          label="Default agent"
          description="Auto tries the allowed agents in order"
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
              <MenuItem key={agent} value={agent}>
                {agentLabel(agent)}
              </MenuItem>
            ))}
          </Select>
        </Row>
      )}
      <Stack spacing={1} sx={{ py: 0.75 }} data-call-targets>
        {targets.map(({ agent, presets }) => {
          const target = calls.targets.find((candidate) =>
            candidate.agent === agent
          );
          return (
            <CallTargetCard
              key={agent}
              agent={agent}
              presets={presets}
              allowed={target !== undefined}
              preset={target?.preset}
              disabled={disabled || !calls.enabled}
              onAllowed={(allowedNow): void =>
                setCalls({
                  targets: withTarget(
                    calls.targets,
                    agent,
                    allowedNow,
                    target?.preset,
                  ),
                })}
              onPreset={(preset): void =>
                setCalls({
                  targets: withTarget(calls.targets, agent, true, preset),
                })}
            />
          );
        })}
      </Stack>
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
          caller={value.agent}
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
            caller={current.agent}
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
