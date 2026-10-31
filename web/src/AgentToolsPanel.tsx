import React, { useState } from "react";
import { desktopSize, glyphSize } from "./surface/desktopSize";
import Check from "@mui/icons-material/Check";
import ExpandMore from "@mui/icons-material/ExpandMore";
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
  DEFAULT_REVIEW_PREFERENCES,
  fetchAgentTools,
  fetchSessionTools,
  normalizedDefault,
  overridden,
  overrideFor,
  saveAgentTools,
  saveSessionTools,
  subscribeSessionTools,
  useSessionToolsSaving,
  SESSION_LIMIT_CHOICES,
  callableTargets,
  choiceFor,
  choiceValues,
  type TargetChoice,
  type ToolOption,
  type ToolPreset,
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

type Preset = ToolPreset;

/** One recommended choice inside a target card: a compact selectable row in
 * the Run configuration preset material. */
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
        minHeight: 44,
        px: 1.25,
        py: 0.5,
        gap: 1,
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
          <Typography variant="body2" noWrap sx={{ fontWeight: 650 }}>
            {name}
          </Typography>
          {tag && (
            <Chip
              label={tag}
              size="small"
              color="primary"
              variant="outlined"
              sx={{ height: 18, fontSize: "0.625rem" }}
            />
          )}
        </Stack>
        {detail && (
          <Typography
            variant="caption"
            color="text.secondary"
            noWrap
            component="div"
          >
            {detail}
          </Typography>
        )}
      </Box>
      <Box
        aria-hidden
        sx={{ width: desktopSize(18), display: "grid", placeItems: "center", flexShrink: 0 }}
      >
        {selected && <Check sx={{ fontSize: glyphSize(18) }} />}
      </Box>
    </ButtonBase>
  );
}

function SectionLabel(
  { children }: { children: React.ReactNode },
): React.JSX.Element {
  return (
    <Typography
      variant="overline"
      color="text.secondary"
      sx={{ display: "block", lineHeight: 1.6, letterSpacing: 0.6, pt: 0.5 }}
    >
      {children}
    </Typography>
  );
}

/** What a target's calls run with, in a few words for the folded card. */
function choiceSummary(
  choice: TargetChoice | undefined,
  presets: readonly Preset[],
  options: readonly ToolOption[],
): string {
  if (choice?.preset) {
    return presets.find((preset) => preset.id === choice.preset)?.name ??
      choice.preset;
  }
  if (choice?.options) {
    const parts = options
      .filter((option) => choice.options?.[option.id] !== undefined)
      .map((option) => {
        const value = choice.options![option.id]!;
        return option.choices.find((c) => c.value === value)?.name ?? value;
      });
    return `Custom · ${parts.join(" · ")}`;
  }
  return "Agent default";
}

/** A callable agent, folded to one row: its name, what its calls run with
 * and its switch. Unfolded, the recommended presets come first; the finer
 * settings below them set each option directly. One card per Provider. */
function CallTargetCard(
  { agent, presets, options, allowed, choice, disabled, onAllowed, onChoice }: {
    agent: string;
    presets: readonly Preset[];
    options: readonly ToolOption[];
    allowed: boolean;
    choice: TargetChoice | undefined;
    disabled: boolean;
    onAllowed: (allowed: boolean) => void;
    onChoice: (choice: TargetChoice) => void;
  },
): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const label = agentLabel(agent);
  const values = choiceValues(choice, presets);
  const editable = !disabled && allowed;
  const setValue = (id: string, value: string): void => {
    const next = { ...values };
    if (value) next[id] = value;
    else delete next[id];
    onChoice(choiceFor(next, presets));
  };
  return (
    <Box
      data-call-target={agent}
      sx={{
        border: 1,
        borderColor: allowed && !disabled ? "primary.main" : "divider",
        borderRadius: 1.5,
        bgcolor: (theme) => alpha(theme.palette.background.default, 0.34),
        overflow: "hidden",
      }}
    >
      <Stack direction="row" alignItems="center" sx={{ pr: 1 }}>
        <ButtonBase
          aria-expanded={open}
          aria-label={`${label} call settings`}
          onClick={(): void => setOpen((value) => !value)}
          sx={{
            flex: 1,
            minWidth: 0,
            minHeight: 56,
            pl: 1.5,
            pr: 1,
            gap: 1.25,
            justifyContent: "flex-start",
            textAlign: "left",
            opacity: disabled ? 0.6 : 1,
          }}
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
              {allowed ? choiceSummary(choice, presets, options) : "Not called"}
            </Typography>
          </Box>
          <ExpandMore
            fontSize="small"
            sx={{
              color: "text.secondary",
              transition: "transform 160ms",
              transform: open ? "rotate(180deg)" : "none",
            }}
          />
        </ButtonBase>
        <Switch
          checked={allowed}
          disabled={disabled}
          onChange={(_, checked): void => onAllowed(checked)}
          slotProps={{ input: { "aria-label": `Allow calls to ${label}` } }}
        />
      </Stack>
      <Collapse in={open} unmountOnExit>
        <Box sx={{ px: 1.5, pb: 1.5 }}>
          <SectionLabel>Recommended</SectionLabel>
          <Stack
            role="radiogroup"
            aria-label={`${label} model and reasoning`}
            spacing={0.75}
          >
            <PresetChoice
              name="Agent default"
              detail={`${label}'s own default model and reasoning`}
              selected={!choice?.preset && !choice?.options}
              disabled={!editable}
              onSelect={(): void => onChoice({})}
            />
            {presets.map((candidate) => (
              <PresetChoice
                key={candidate.id}
                name={candidate.name}
                detail={candidate.detail}
                tag={candidate.is_default ? "Provider default" : undefined}
                selected={candidate.id === choice?.preset}
                disabled={!editable}
                onSelect={(): void => onChoice({ preset: candidate.id })}
              />
            ))}
          </Stack>
          {options.length > 0 && (
            <>
              <SectionLabel>Custom</SectionLabel>
              <Stack spacing={1} data-call-target-options>
                {options.map((option) => (
                  <Stack
                    key={option.id}
                    direction="row"
                    alignItems="center"
                    spacing={1.5}
                  >
                    <Typography variant="body2" sx={{ flex: 1, minWidth: 0 }}>
                      {option.name}
                    </Typography>
                    <Select
                      size="small"
                      displayEmpty
                      value={values[option.id] ?? ""}
                      disabled={!editable}
                      onChange={(event): void =>
                        setValue(option.id, String(event.target.value))}
                      inputProps={{ "aria-label": `${label} ${option.name}` }}
                      sx={{ minWidth: 168, maxWidth: "60%" }}
                    >
                      <MenuItem value="">
                        <Typography component="span" color="text.secondary">
                          Agent default
                        </Typography>
                      </MenuItem>
                      {option.choices.map((item) => (
                        <MenuItem key={item.value} value={item.value}>
                          {item.name}
                        </MenuItem>
                      ))}
                    </Select>
                  </Stack>
                ))}
              </Stack>
            </>
          )}
        </Box>
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
      <Heading>Review preference</Heading>
      {(["claude", "gpt"] as const).map((family) => {
        const review = calls.review ?? DEFAULT_REVIEW_PREFERENCES;
        const models: [string, string] = family === "claude" ? ["opus", "fable"] : ["astra", "sol"];
        const label = (model: string) => ({ opus: "Opus", fable: "Fable", astra: "Astra", sol: "SOL" })[model];
        const choices = [models, [...models].reverse(), [models[0]], [models[1]]];
        return (
          <Row key={family} label={family === "claude" ? "Claude reviewer" : "GPT reviewer"}
            description="Preferred model first; alternatives only when quota or availability requires it">
            <Select size="small" value={review[family].join(",")} disabled={disabled}
              inputProps={{ "aria-label": `${family === "claude" ? "Claude" : "GPT"} review preference` }}
              onChange={(event) => setCalls({ review: { ...review, [family]: event.target.value.split(",") } })}>
              {choices.map((order) => <MenuItem key={order.join(",")} value={order.join(",")}>
                {order.length === 1 ? `${label(order[0]!)} only` : order.map(label).join(" → ")}
              </MenuItem>)}
            </Select>
          </Row>
        );
      })}
      <Heading>Agent calls</Heading>
      <Row
        label="Allow calls"
        description={`Let this agent ask ${
          names.length > 0 ? names.join(" or ") : "another agent"
        } for a read-only review when it judges a change risky. Most tasks need none; off costs no tokens.`}
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
        {targets.map(({ agent, presets, options = [] }) => {
          const target = calls.targets.find((candidate) =>
            candidate.agent === agent
          );
          return (
            <CallTargetCard
              key={agent}
              agent={agent}
              presets={presets}
              options={options}
              allowed={target !== undefined}
              choice={target}
              disabled={disabled || !calls.enabled}
              onAllowed={(allowedNow): void =>
                setCalls({
                  targets: withTarget(
                    calls.targets,
                    agent,
                    allowedNow,
                    target,
                  ),
                })}
              onChoice={(choice): void =>
                setCalls({
                  targets: withTarget(calls.targets, agent, true, choice),
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
      <Heading>Memory</Heading>
      <Row label="Memory tools" description="Search and record shared memory">
        <Switch
          checked={value.memory.tools}
          disabled={disabled}
          onChange={(_, checked): void =>
            onChange({ ...value, memory: { ...value.memory, tools: checked } })}
          slotProps={{ input: { "aria-label": "Memory tools" } }}
        />
      </Row>
      <Row
        label="Automatic recall"
        description="Relevant memory before each turn"
      >
        <Switch
          checked={value.memory.recall}
          disabled={disabled}
          onChange={(_, checked): void =>
            onChange({
              ...value,
              memory: { ...value.memory, recall: checked },
            })}
          slotProps={{ input: { "aria-label": "Memory automatic recall" } }}
        />
      </Row>
      <Typography
        variant="caption"
        color="text.secondary"
        sx={{ display: "block", pb: 1 }}
      >
        The memory backend is the Plugin installed on the session's runtime
        Machine. Changes apply when the session's agent next starts.
      </Typography>
    </Box>
  );
}

/** A session's tools: its agent kind's defaults with this session's changes. */
export function SessionToolsSection(
  { sessionId, disabled = false }: { sessionId: string; disabled?: boolean },
): React.JSX.Element {
  const sharedSaving = useSessionToolsSaving(sessionId);
  const { value, error, saving, save } = useRemoteDocument(
    () => fetchSessionTools(sessionId),
    sessionId,
    (receive) => subscribeSessionTools(sessionId, receive),
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
                disabled={saving || sharedSaving || disabled}
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
          disabled={saving || sharedSaving || disabled}
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
