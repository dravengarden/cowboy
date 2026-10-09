import {
  Box,
  Button,
  ButtonBase,
  Chip,
  CircularProgress,
  Collapse,
  Divider,
  IconButton,
  Stack,
  Tooltip,
  Typography,
} from "@mui/material";
import {
  AccountTree,
  ArrowBack,
  ExpandLess,
  ExpandMore,
  OpenInNew,
  StopCircleOutlined,
} from "@mui/icons-material";
import { memo, type ReactNode, useMemo, useState } from "react";
import { useStore } from "@cowboy/state-store";
import { ProviderIcon } from "./ProviderIcon";
import { Markdown } from "./Markdown";
import { ConfirmSheet, Sheet } from "./Sheet";
import { setActiveSessionId } from "./controlPlane";
import { useReliableTouchTap } from "./useReliableTouchTap";
import { desktopSurfaceSx } from "./desktop/DesktopEmbeddedControl";
import {
  mobileComposerPanelFrameSx,
  mobileComposerPanelHeaderMinHeight,
} from "./mobileComposerPrimitives";
import {
  composerStackExpandedStore,
  toggleComposerStackPanel,
} from "./composerStackAccordion";
import {
  callActive,
  callGroup,
  callsOverview,
  callStateLabel,
  callTitle,
  callTone,
  cancelCall,
  elapsedLabel,
  machineName,
  type ManagedCallDetail,
  type ManagedCallSummary,
  providerLabel,
  reviewFindings,
  reviewSummary,
  reviewVerdict,
  shortRevision,
  useManagedCall,
  useManagedCalls,
} from "./managedCalls";

function StateChip({ call }: { call: ManagedCallSummary }): React.JSX.Element {
  const tone = callTone(call.state);
  return (
    <Chip
      size="small"
      variant={tone === "active" ? "outlined" : "filled"}
      color={tone === "success"
        ? "success"
        : tone === "error"
        ? "error"
        : tone === "active"
        ? "primary"
        : "default"}
      label={callStateLabel(call)}
      icon={tone === "active"
        ? <CircularProgress size="0.75rem" thickness={5} />
        : undefined}
      sx={{ height: 22, fontWeight: 600, "& .MuiChip-label": { px: 0.75 } }}
    />
  );
}

/** Review verdicts are separate from execution success: a completed call can
 * still need attention, and a failed call has no verdict at all. */
function VerdictChip(
  { verdict, findings }: {
    verdict?: string | null;
    findings?: number | undefined;
  },
): React.JSX.Element | null {
  if (!verdict && findings === undefined) return null;
  const attention = verdict !== null && verdict !== undefined &&
    verdict !== "approve";
  return (
    <Chip
      size="small"
      variant="outlined"
      color={attention ? "warning" : "default"}
      label={[
        verdict ? verdict.replace(/-/g, " ") : null,
        findings !== undefined
          ? `${findings} finding${findings === 1 ? "" : "s"}`
          : null,
      ].filter(Boolean).join(" · ")}
      sx={{ height: 22, "& .MuiChip-label": { px: 0.75 } }}
    />
  );
}

function placementLabel(
  call: Pick<ManagedCallSummary, "runtime_machine_id" | "placement">,
): string {
  const runtime = call.runtime_machine_id;
  const target = call.placement.machine_id;
  return runtime && runtime !== target
    ? `${machineName(runtime)} → ${machineName(target)}`
    : machineName(target);
}

function CallRow({
  call,
  selected,
  desktop,
  now,
  onSelect,
}: {
  call: ManagedCallSummary;
  selected: boolean;
  desktop: boolean;
  now: number;
  onSelect: () => void;
}): React.JSX.Element {
  const tap = useReliableTouchTap<HTMLButtonElement>(onSelect);
  const end = callActive(call.state) ? now : call.updated_at_ms;
  return (
    <ButtonBase
      {...tap}
      aria-current={selected ? "true" : undefined}
      aria-label={`${providerLabel(call.provider)} ${callTitle(call)}: ${
        callStateLabel(call)
      }`}
      {...(desktop
        ? { "data-desktop-item": `call-${call.call_id}`, tabIndex: -1 }
        : {})}
      sx={{
        width: "100%",
        textAlign: "left",
        justifyContent: "flex-start",
        borderRadius: 1,
        px: 1,
        py: desktop ? 0.75 : 1.25,
        minHeight: desktop ? 44 : 56,
        bgcolor: selected ? "action.selected" : "transparent",
        "&:focus-visible": {
          outline: "2px solid",
          outlineColor: "primary.main",
        },
      }}
    >
      <Stack
        direction="row"
        spacing={1}
        alignItems="center"
        sx={{ width: "100%", minWidth: 0 }}
      >
        <ProviderIcon
          provider={call.provider}
          sx={{ fontSize: "1.25rem", flexShrink: 0 }}
        />
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography
            variant="body2"
            noWrap
            sx={{ fontWeight: 600, textTransform: "capitalize" }}
          >
            {callTitle(call)}
          </Typography>
          <Typography
            variant="caption"
            color="text.secondary"
            noWrap
            component="div"
          >
            {providerLabel(call.provider)} · {placementLabel(call)} ·{" "}
            {elapsedLabel(call.created_at_ms, end)}
          </Typography>
        </Box>
        <Stack
          direction={desktop ? "row" : "column"}
          spacing={0.5}
          alignItems="flex-end"
          sx={{ flexShrink: 0 }}
        >
          <StateChip call={call} />
          <VerdictChip
            verdict={call.verdict ?? null}
            findings={call.finding_count}
          />
        </Stack>
      </Stack>
    </ButtonBase>
  );
}

function DetailRow(
  { label, value, mono = false }: {
    label: string;
    value: ReactNode;
    mono?: boolean;
  },
): React.JSX.Element {
  return (
    <Stack direction="row" spacing={1.5} sx={{ minWidth: 0 }}>
      <Typography
        variant="caption"
        color="text.secondary"
        sx={{ width: 112, flexShrink: 0 }}
      >
        {label}
      </Typography>
      <Typography
        variant="caption"
        component="div"
        sx={{
          minWidth: 0,
          overflowWrap: "anywhere",
          fontFamily: mono ? "monospace" : undefined,
        }}
      >
        {value}
      </Typography>
    </Stack>
  );
}

function ResultSection(
  { detail, touch }: { detail: ManagedCallDetail; touch: boolean },
): React.JSX.Element {
  const result = detail.result;
  if (!result) {
    return (
      <Typography variant="body2" color="text.secondary">
        {callActive(detail.state)
          ? "The child is still working. The result appears here when its turn ends."
          : "No result was captured."}
      </Typography>
    );
  }
  const structured = result.structured ?? null;
  const findings = reviewFindings(structured);
  const verdict = reviewVerdict(structured);
  const summary = reviewSummary(structured);
  if (findings !== null || verdict !== null) {
    return (
      <Stack spacing={1}>
        <Stack direction="row" spacing={1} alignItems="center">
          <Typography variant="overline">Review result</Typography>
          <VerdictChip verdict={verdict} findings={findings?.length} />
        </Stack>
        {summary && <Typography variant="body2">{summary}</Typography>}
        {(findings ?? []).map((finding, index) => (
          <Box
            key={index}
            sx={{ border: 1, borderColor: "divider", borderRadius: 1, p: 1 }}
          >
            <Stack
              direction="row"
              spacing={1}
              alignItems="center"
              sx={{ mb: 0.5 }}
            >
              {finding.severity && (
                <Chip
                  size="small"
                  label={finding.severity}
                  color={finding.severity === "critical" ||
                      finding.severity === "high"
                    ? "error"
                    : finding.severity === "medium"
                    ? "warning"
                    : "default"}
                  sx={{ height: 20, textTransform: "capitalize" }}
                />
              )}
              <Typography variant="body2" sx={{ fontWeight: 600, minWidth: 0 }}>
                {finding.title}
              </Typography>
            </Stack>
            {finding.location && (
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ fontFamily: "monospace", overflowWrap: "anywhere" }}
              >
                {finding.location}
              </Typography>
            )}
            {finding.body && <Markdown text={finding.body} touchWrap={touch} />}
          </Box>
        ))}
        <Typography variant="caption" color="text.secondary">
          Accepting, fixing or dismissing these findings is recorded by the
          calling workflow, not by Cowboy.
        </Typography>
      </Stack>
    );
  }
  if (structured !== null) {
    return (
      <Markdown
        text={"```json\n" + JSON.stringify(structured, null, 2) + "\n```"}
        touchWrap={touch}
      />
    );
  }
  return <Markdown text={result.text ?? ""} touchWrap={touch} />;
}

function CallDetailView({
  parent,
  call,
  desktop,
  onOpenChild,
  onBack,
}: {
  parent: string;
  call: ManagedCallSummary;
  desktop: boolean;
  onOpenChild: (child: string) => void;
  onBack?: (() => void) | undefined;
}): React.JSX.Element {
  const { detail, error } = useManagedCall(
    parent,
    call.call_id,
    call.updated_at_ms,
  );
  const [confirmStop, setConfirmStop] = useState(false);
  const [stopError, setStopError] = useState<string | null>(null);
  const stoppable = callActive(call.state) && !call.cancel_requested;
  const labels = Object.entries(call.labels);
  return (
    <Stack spacing={1.25} sx={{ minWidth: 0 }}>
      <Stack direction="row" spacing={1} alignItems="center">
        {onBack && (
          <IconButton
            aria-label="Back to calls"
            onClick={onBack}
            size={desktop ? "small" : "medium"}
          >
            <ArrowBack fontSize="small" />
          </IconButton>
        )}
        <ProviderIcon provider={call.provider} sx={{ fontSize: "1.375rem" }} />
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography
            variant="subtitle2"
            noWrap
            sx={{ textTransform: "capitalize" }}
          >
            {callTitle(call)}
          </Typography>
          <Typography
            variant="caption"
            color="text.secondary"
            noWrap
            component="div"
          >
            {providerLabel(call.provider)} {call.provider_version ?? ""}{" "}
            · read-only {call.purpose}
          </Typography>
        </Box>
        <StateChip call={call} />
      </Stack>
      <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
        <Button
          size="small"
          variant="outlined"
          startIcon={<OpenInNew fontSize="small" />}
          onClick={(): void => onOpenChild(call.child_session_id)}
          sx={{ minHeight: desktop ? 32 : 44 }}
        >
          Open conversation
        </Button>
        {stoppable && (
          <Button
            size="small"
            color="error"
            variant="outlined"
            startIcon={<StopCircleOutlined fontSize="small" />}
            onClick={(): void => setConfirmStop(true)}
            sx={{ minHeight: desktop ? 32 : 44 }}
          >
            Stop call
          </Button>
        )}
      </Stack>
      {stopError && (
        <Typography variant="caption" color="error">{stopError}</Typography>
      )}
      <Divider />
      <Typography variant="overline">Execution</Typography>
      <Stack spacing={0.5}>
        <DetailRow
          label="Status"
          value={call.cancel_requested && callActive(call.state)
            ? "Stop requested; waiting for the child to confirm"
            : callStateLabel(call)}
        />
        {call.error && (
          <DetailRow
            label="Cause"
            value={call.error.detail
              ? `${call.error.code}: ${call.error.detail}`
              : call.error.code}
          />
        )}
        <DetailRow
          label="AI runtime"
          value={machineName(call.runtime_machine_id)}
        />
        <DetailRow
          label="Runs on"
          value={machineName(call.placement.machine_id)}
        />
        <DetailRow
          label="Input snapshot"
          value={shortRevision(call.input_revision)}
          mono
        />
        <DetailRow
          label="Started"
          value={new Date(call.created_at_ms).toLocaleString()}
        />
        <DetailRow
          label="Updated"
          value={new Date(call.updated_at_ms).toLocaleString()}
        />
        {labels.length > 0 && (
          <DetailRow
            label="Labels"
            value={labels.map(([key, value]) => `${key}: ${value}`).join(" · ")}
          />
        )}
        <DetailRow label="Call" value={call.call_id} mono />
        <DetailRow label="Request" value={call.request_id} mono />
      </Stack>
      <Divider />
      {error && !detail && (
        <Typography variant="caption" color="error">{error}</Typography>
      )}
      {detail
        ? <ResultSection detail={detail} touch={!desktop} />
        : !error && <CircularProgress size="1rem" />}
      <ConfirmSheet
        open={confirmStop}
        onClose={(): void => setConfirmStop(false)}
        title="Stop this call?"
        actions={
          <Stack
            direction="row"
            spacing={1}
            justifyContent="flex-end"
            sx={{ width: "100%" }}
          >
            <Button onClick={(): void => setConfirmStop(false)}>
              Keep running
            </Button>
            <Button
              color="error"
              variant="contained"
              onClick={(): void => {
                setConfirmStop(false);
                void cancelCall(parent, call.call_id).catch(
                  (reason: unknown) => {
                    setStopError(
                      reason instanceof Error
                        ? reason.message
                        : "Stop was not recorded",
                    );
                  },
                );
              }}
            >
              Stop call
            </Button>
          </Stack>
        }
      >
        <Typography variant="body2">
          Cowboy asks the child to stop its current turn. If the child finishes
          first, its real result is kept.
        </Typography>
      </ConfirmSheet>
    </Stack>
  );
}

function CallList({
  calls,
  selected,
  desktop,
  onSelect,
}: {
  calls: readonly ManagedCallSummary[];
  selected: string | null;
  desktop: boolean;
  onSelect: (call: string) => void;
}): React.JSX.Element {
  const now = Date.now();
  // Group by the caller's explicit group label; never by parsing prose.
  const groups = useMemo(() => {
    const map = new Map<string, ManagedCallSummary[]>();
    for (const call of calls) {
      const key = callGroup(call) ?? "";
      map.set(key, [...(map.get(key) ?? []), call]);
    }
    return [...map.entries()];
  }, [calls]);
  return (
    <Stack spacing={0.25}>
      {groups.map(([group, members]) => (
        <Box key={group}>
          {group && (
            <Typography
              variant="caption"
              color="text.secondary"
              sx={{ px: 1, fontWeight: 600 }}
            >
              {group}
            </Typography>
          )}
          {members.map((call) => (
            <CallRow
              key={call.call_id}
              call={call}
              selected={call.call_id === selected}
              desktop={desktop}
              now={now}
              onSelect={(): void => onSelect(call.call_id)}
            />
          ))}
        </Box>
      ))}
    </Stack>
  );
}

/** Parent-scoped managed calls above the Prompt: a compact summary that opens
 * an inline list and detail on Desktop and a full-height page on Mobile. */
export const ManagedCallsDock = memo(function ManagedCallsDock({
  sessionId,
  desktop,
  shortcut,
}: {
  sessionId: string;
  desktop: boolean;
  shortcut?: ReactNode;
}): React.JSX.Element | null {
  const { calls, observedAt, error } = useManagedCalls(sessionId);
  const expanded = useStore(composerStackExpandedStore()) === "calls";
  const [selected, setSelected] = useState<string | null>(null);
  const [mobileOpen, setMobileOpen] = useState(false);
  const toggle = (): void => {
    if (desktop) toggleComposerStackPanel("calls");
    else setMobileOpen(true);
  };
  const toggleTap = useReliableTouchTap<HTMLButtonElement>(toggle);
  if (calls.length === 0) return null;
  const overview = callsOverview(calls);
  const selectedCall = calls.find((call) => call.call_id === selected) ?? null;
  const stale = error !== null && observedAt !== null;
  const openChild = (child: string): void => {
    setMobileOpen(false);
    setActiveSessionId(child);
  };
  const summary = overview.active > 0
    ? `${overview.activeTitle ?? "Working"}${
      overview.active > 1 ? ` +${overview.active - 1}` : ""
    }`
    : overview.failed > 0
    ? `${overview.failed} failed`
    : "All calls finished";
  return (
    <Box
      data-desktop-calls-surface={desktop ? "true" : undefined}
      sx={{
        ...(desktop
          ? desktopSurfaceSx({ interactive: false, focusWithin: true })
          : mobileComposerPanelFrameSx),
        mb: desktop ? 1 : 0,
        bgcolor: "background.default",
        // Own the text color and typography: rows and details are rendered in
        // ButtonBase/Sheet surfaces that would otherwise inherit the page's.
        color: "text.primary",
        typography: "body2",
        overflow: "hidden",
      }}
    >
      <ButtonBase
        {...toggleTap}
        aria-label={desktop
          ? (expanded ? "Collapse calls" : "Expand calls")
          : "Open calls"}
        aria-expanded={desktop ? expanded : mobileOpen}
        sx={{
          width: "100%",
          justifyContent: "flex-start",
          textAlign: "left",
          px: 1,
          py: 0.5,
          minHeight: desktop ? 40 : mobileComposerPanelHeaderMinHeight,
          "@media (pointer: coarse)": {
            minHeight: mobileComposerPanelHeaderMinHeight,
          },
          touchAction: "manipulation",
        }}
      >
        <Stack
          direction="row"
          alignItems="center"
          spacing={1}
          sx={{ width: "100%", minWidth: 0 }}
        >
          {desktop
            ? (expanded
              ? <ExpandLess fontSize="small" sx={{ color: "text.secondary" }} />
              : (
                <ExpandMore fontSize="small" sx={{ color: "text.secondary" }} />
              ))
            : <AccountTree fontSize="small" sx={{ color: "text.secondary" }} />}
          <Typography variant="overline" sx={{ lineHeight: 1.4 }}>
            Calls
          </Typography>
          {overview.active > 0 && (
            <CircularProgress size="0.875rem" thickness={5} />
          )}
          <Typography
            variant="body2"
            noWrap
            sx={{
              flex: 1,
              minWidth: 0,
              color: overview.failed > 0 && overview.active === 0
                ? "error.main"
                : "text.secondary",
              textTransform: "capitalize",
            }}
          >
            {summary}
          </Typography>
          {stale && (
            <Tooltip
              title={`Last updated ${
                new Date(observedAt).toLocaleTimeString()
              }`}
            >
              <Typography variant="caption" color="warning.main">
                offline
              </Typography>
            </Tooltip>
          )}
          {overview.findings > 0 && (
            <Typography
              variant="caption"
              color="warning.main"
              sx={{ fontWeight: 600 }}
            >
              {overview.findings} finding{overview.findings === 1 ? "" : "s"}
            </Typography>
          )}
          {shortcut}
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ fontWeight: 600, fontVariantNumeric: "tabular-nums" }}
          >
            {overview.total - overview.active}/{overview.total}
          </Typography>
        </Stack>
      </ButtonBase>
      {desktop && (
        <Collapse in={expanded}>
          {
            /* The Prompt column is narrow beside the Conversation pane, so the
              list and the selected call's detail stack inside one bounded
              scroller; the parent transcript stays visible next to it. */
          }
          <Stack
            spacing={1}
            data-desktop-aux-list="true"
            sx={{
              p: 1,
              maxHeight: "min(56vh, 680px)",
              overflowY: expanded ? "auto" : "hidden",
              overscrollBehavior: "contain",
            }}
          >
            <CallList
              calls={calls}
              selected={selected}
              desktop
              onSelect={setSelected}
            />
            {selectedCall && (
              <Box sx={{ borderTop: 1, borderColor: "divider", pt: 1 }}>
                <CallDetailView
                  parent={sessionId}
                  call={selectedCall}
                  desktop
                  onOpenChild={openChild}
                  onBack={(): void =>
                    setSelected(null)}
                />
              </Box>
            )}
          </Stack>
        </Collapse>
      )}
      {!desktop && (
        <Sheet
          open={mobileOpen}
          onClose={(): void =>
            setMobileOpen(false)}
          title={selectedCall ? "Call" : "Calls"}
          forceSheet
          cover
          portal
        >
          <Box sx={{ pb: 2, color: "text.primary", typography: "body2" }}>
            {selectedCall
              ? (
                <CallDetailView
                  parent={sessionId}
                  call={selectedCall}
                  desktop={false}
                  onOpenChild={openChild}
                  onBack={(): void => setSelected(null)}
                />
              )
              : (
                <CallList
                  calls={calls}
                  selected={null}
                  desktop={false}
                  onSelect={setSelected}
                />
              )}
          </Box>
        </Sheet>
      )}
    </Box>
  );
});

/** Shown instead of the Prompt in a managed child: one controller owns it. */
export function ManagedChildNotice(
  { parent }: { parent: string },
): React.JSX.Element {
  return (
    <Box
      sx={{
        m: 1,
        p: 1.5,
        border: 1,
        borderColor: "divider",
        borderRadius: 2,
        bgcolor: "background.default",
      }}
    >
      <Stack direction="row" spacing={1.5} alignItems="center">
        <AccountTree fontSize="small" sx={{ color: "text.secondary" }} />
        <Typography variant="body2" sx={{ flex: 1 }}>
          This read-only child conversation is controlled by its parent's
          managed call.
        </Typography>
        <Button
          size="small"
          variant="outlined"
          onClick={(): void => setActiveSessionId(parent)}
          sx={{ minHeight: 44 }}
        >
          Open parent
        </Button>
      </Stack>
    </Box>
  );
}
