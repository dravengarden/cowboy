import {
  COMPOSER_STACK_DISCLOSURE_TIMEOUT,
  composerStackDisclosureSx,
  swallowStrayDisclosureClick,
} from "./composerStackDisclosure";
import {
  alpha,
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
  useMediaQuery,
  useTheme,
} from "@mui/material";
import { desktopSize } from "./surface/desktopSize";
import {
  AccountTree,
  ChevronRight,
  Close,
  ExpandMore,
  InfoOutlined,
  OpenInNew,
  VerticalAlignBottom,
  VerticalAlignTop,
  StopCircleOutlined,
} from "@mui/icons-material";
import {
  type KeyboardEvent,
  memo,
  type ReactNode,
  useEffect,
  useMemo,
  useState,
} from "react";
import { DesktopShortcutBar } from "./desktop/DesktopShortcutBar";
import { desktopScrollbarSx } from "./desktop/desktopScrollbar";
import { HintTooltip } from "./HintTooltip";
import { CallTranscript } from "./CallTranscript";
import { CallCleanup } from "./CallCleanup";
import { requestScrollToTop, requestStickToBottom } from "./stickyStore";
import { useStore } from "@cowboy/state-store";
import { ProviderIcon } from "./ProviderIcon";
import { AccountBadge } from "./AccountBadge";
import { accountIdentity } from "./accountIdentity";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { Markdown } from "./Markdown";
import { ConfirmSheet, Sheet } from "./Sheet";
import { setActiveSessionId } from "./controlPlane";
import { useReliableTouchTap } from "./useReliableTouchTap";
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
  callAgeLabel,
  callModelLabel,
  callExecutionParameters,
  callGroup,
  callsOverview,
  callStateLabel,
  callTitle,
  callPreviewTitles,
  callTone,
  callUsageRows,
  callConversationId,
  cancelCall,
  callTimesTitle,
  elapsedLabel,
  machineName,
  type ManagedCallDetail,
  type ManagedCallSummary,
  providerLabel,
  reviewFindings,
  startedLabel,
  reviewSummary,
  reviewVerdict,
  shortRevision,
  useManagedCall,
  useManagedCalls,
  useCallClock,
} from "./managedCalls";

function StateChip(
  { call, desktop = false }: { call: ManagedCallSummary; desktop?: boolean },
): React.JSX.Element {
  const tone = callTone(call.state);
  const color = tone === "success"
    ? "success"
    : tone === "error"
    ? "error"
    : tone === "active"
    ? "primary"
    : "default";
  // Desktop: the same quiet tinted badge as its verdict, with a hairline
  // spinner while the call is live. An outlined chip reads as a button.
  if (desktop) {
    return (
      <Badge
        label={callStateLabel(call)}
        color={color}
        icon={tone === "active"
          ? <CircularProgress size={desktopSize(10)} thickness={5} color="inherit" />
          : undefined}
      />
    );
  }
  return (
    <Chip
      size="small"
      variant={tone === "active" ? "outlined" : "filled"}
      color={color}
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
  { verdict, findings, desktop = false }: {
    verdict?: string | null;
    findings?: number | undefined;
    desktop?: boolean;
  },
): React.JSX.Element | null {
  if (!verdict && findings === undefined) return null;
  const attention = verdict !== null && verdict !== undefined &&
    verdict !== "approve";
  const label = [
    verdict ? verdict.replace(/-/g, " ") : null,
    findings !== undefined
      ? `${findings} finding${findings === 1 ? "" : "s"}`
      : null,
  ].filter(Boolean).join(" · ");
  if (desktop) {
    return <Badge label={label} color={attention ? "warning" : "default"} />;
  }
  return (
    <Chip
      size="small"
      variant="outlined"
      color={attention ? "warning" : "default"}
      label={label}
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

/** The one badge a row needs: a review's verdict already says it finished,
 * so a completed call with a verdict shows only the verdict. */
function CallOutcomeBadge(
  { call }: { call: ManagedCallSummary },
): React.JSX.Element {
  const hasOutcome = Boolean(call.verdict) || call.finding_count !== undefined;
  if (call.state === "completed" && hasOutcome) {
    const attention = call.verdict !== undefined && call.verdict !== null &&
      call.verdict !== "approve";
    const label = [
      call.verdict ? call.verdict.replace(/-/g, " ") : null,
      call.finding_count !== undefined
        ? `${call.finding_count} finding${call.finding_count === 1 ? "" : "s"}`
        : null,
    ].filter(Boolean).join(" · ");
    return <Badge label={label} color={attention ? "warning" : "success"} />;
  }
  return (
    <Badge
      label={callStateLabel(call)}
      color={stateBadgeColor(call)}
      icon={callActive(call.state)
        ? <CircularProgress size={desktopSize(10)} thickness={5} color="inherit" />
        : undefined}
    />
  );
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
        display: "grid",
        // Icon column, then title/outcome over one metadata line. The icon
        // sits on the title's baseline row instead of floating mid-row.
        gridTemplateColumns: "auto minmax(0, 1fr)",
        columnGap: 1,
        rowGap: 0.25,
        alignItems: "center",
        borderRadius: 1,
        px: 1,
        py: desktop ? 0.625 : 1,
        minHeight: desktop ? 40 : 56,
        bgcolor: selected ? "action.selected" : "transparent",
        "&:hover": { bgcolor: selected ? "action.selected" : "action.hover" },
        "&:focus-visible": {
          outline: "2px solid",
          outlineColor: "primary.main",
        },
      }}
    >
      {/* Explicit cells: an icon that renders nothing must not shift the
          title into the icon column. */}
      <Box sx={{ gridColumn: 1, gridRow: 1, display: "inline-flex", width: "1rem" }}>
        <ProviderIcon provider={call.provider} sx={{ fontSize: "1rem" }} />
      </Box>
      <Stack direction="row" alignItems="center" spacing={0.75} sx={{ gridColumn: 2, gridRow: 1, minWidth: 0 }}>
        <Typography
          variant="body2"
          noWrap
          sx={{ flex: 1, minWidth: 0, fontWeight: 600, textTransform: "capitalize" }}
        >
          {callTitle(call)}
        </Typography>
        <CallOutcomeBadge call={call} />
      </Stack>
      <Box
        sx={{
          gridColumn: 2,
          display: "flex",
          alignItems: "center",
          gap: 0.75,
          minWidth: 0,
          typography: "caption",
          color: "text.secondary",
        }}
      >
        <CallAccountBadge call={call} />
        <Box
          component="span"
          sx={{ minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
          title={`${callModelLabel(call)} · ${placementLabel(call)}`}
        >
          {callModelLabel(call)} · {placementLabel(call)}
        </Box>
        <Box
          component="span"
          title={callTimesTitle(call)}
          aria-label={`started ${startedLabel(call.created_at_ms)} · ${
            elapsedLabel(call.created_at_ms, end)
          }`}
          sx={{ flexShrink: 0, ml: "auto", whiteSpace: "nowrap", fontVariantNumeric: "tabular-nums" }}
        >
          {startedLabel(call.created_at_ms)} · {elapsedLabel(call.created_at_ms, end)}
        </Box>
      </Box>
      {call.error && (
        <Typography
          variant="caption"
          color="error"
          component="div"
          noWrap
          sx={{ gridColumn: 2, minWidth: 0 }}
        >
          {call.error.detail ?? call.error.code}
        </Typography>
      )}
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
          <VerdictChip
            verdict={verdict}
            findings={findings?.length}
            desktop={!touch}
          />
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
}: {
  parent: string;
  call: ManagedCallSummary;
  desktop: boolean;
  /** Absent when the child is already the opened session. */
  onOpenChild?: ((child: string) => void) | undefined;
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
        <StateChip call={call} desktop={desktop} />
      </Stack>
      <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
        {onOpenChild && (
          <Button
            size="small"
            variant="outlined"
            startIcon={<OpenInNew fontSize="small" />}
            onClick={(): void => onOpenChild(callConversationId(call))}
            sx={{ minHeight: desktop ? 32 : 44 }}
          >
            Open as session
          </Button>
        )}
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
        <CallAccountDetails call={detail ?? call} />
        {(call.reused_from || detail?.result?.reused_from) && (
          <DetailRow label="Reused result" value={call.reused_from ?? detail?.result?.reused_from?.call_id ?? ""} mono />
        )}
        {callUsageRows(detail?.result?.usage ?? call.usage, call.reused_from ?? detail?.result?.reused_from?.call_id).map((row) => (
          <DetailRow key={row.label} label={row.label} value={row.value} />
        ))}
        <DetailRow label="Model / effort" value={callModelLabel(call)} />
        <DetailRow label="Selected preset" value={call.preset ?? "Provider default"} />
        <DetailRow label="Selection" value={call.labels["selection_reason"] ?? call.selection ?? "Not recorded"} />
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
        {call.labels["review_base"] && (
          <DetailRow label="Review base" value={call.labels["review_base"]} mono />
        )}
        {detail?.result?.review_input?.root && (
          <DetailRow label="Review repository" value={detail.result.review_input.root} mono />
        )}
        <DetailRow
          label="Created"
          value={new Date(call.created_at_ms).toLocaleString()}
        />
        <DetailRow label="Started" value={call.started_at_ms == null ? "Not recorded" : new Date(call.started_at_ms).toLocaleString()} />
        <DetailRow label="Finished" value={call.finished_at_ms == null ? (callActive(call.state) ? "In progress" : "Not recorded") : new Date(call.finished_at_ms).toLocaleString()} />
        <DetailRow label="Elapsed" value={call.started_at_ms == null ? "Not recorded" : elapsedLabel(call.started_at_ms, call.finished_at_ms ?? (callActive(call.state) ? Date.now() : call.updated_at_ms))} />
        <DetailRow label="Time zone" value={Intl.DateTimeFormat().resolvedOptions().timeZone} />
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

/** A call's title as a heading ("Correctness · Round 1"); list rows get the
 * same effect from CSS, but sheet titles are plain strings. */
function displayTitle(call: ManagedCallSummary): string {
  return callTitle(call).replace(
    /(^|[\s·-])(\p{Ll})/gu,
    (_, lead, letter) => `${lead}${letter.toUpperCase()}`,
  );
}

/** A small filled badge in a status color. */
function Badge(
  { label, color, icon }: {
    label: string;
    color: "success" | "error" | "warning" | "primary" | "default";
    icon?: ReactNode;
  },
): React.JSX.Element {
  return (
    <Box
      component="span"
      sx={(theme) => {
        const main = color === "default"
          ? theme.palette.text.secondary
          : theme.palette[color].main;
        return {
          flexShrink: 0,
          px: 0.75,
          height: desktopSize(20),
          display: "inline-flex",
          alignItems: "center",
          gap: 0.5,
          borderRadius: 999,
          fontSize: "0.6875rem",
          fontWeight: 700,
          lineHeight: 1,
          whiteSpace: "nowrap",
          color: main,
          bgcolor: alpha(main, theme.palette.mode === "dark" ? 0.22 : 0.14),
        };
      }}
    >
      {icon}
      {label}
    </Box>
  );
}

function useCallAccount(call: Pick<ManagedCallSummary, "provider" | "provider_account_id">) {
  const { data } = useSubscriptionAccounts();
  return accountIdentity(data, call.provider, call.provider_account_id);
}

function CallAccountBadge({ call }: { call: ManagedCallSummary }): React.JSX.Element {
  return <AccountBadge account={useCallAccount(call)} />;
}

function CallAccountDetails({ call }: { call: Pick<ManagedCallSummary, "provider" | "provider_account_id"> }): React.JSX.Element {
  const account = useCallAccount(call);
  return <DetailRow label="Account" value={<span title={account.description}>{account.label}</span>} />;
}

/** A compact phone header: title above status, placement, time and recorded configuration.
 * It replaces the sheet's grab handle. */
function CallHeader({ call }: { call: ManagedCallSummary }): React.JSX.Element {
  const now = useCallClock();
  const end = callActive(call.state) ? now : call.finished_at_ms ?? call.updated_at_ms;
  const parameters = callExecutionParameters(call).filter(({ label, value }) =>
    value !== "Not recorded" && !(label === "Preset" && call.configuration?.model)
  );
  const configuration = parameters.map(({ value }) => value).join(" · ");
  const tone = callTone(call.state);
  const attention =
    call.verdict !== undefined &&
    call.verdict !== null &&
    call.verdict !== "approve";
  const outcome =
    call.finding_count !== undefined && call.verdict
      ? `${call.finding_count} finding${call.finding_count === 1 ? "" : "s"}`
      : call.verdict
        ? call.verdict.replace(/-/g, " ")
        : null;
  return (
    <Box
      data-call-header={call.call_id}
      sx={{
        display: "flex",
        flexDirection: "column",
        gap: 0.25,
        minWidth: 0,
        pl: 2,
        pr: "max(env(safe-area-inset-right), 16px)",
        py: 0.75,
        borderBottom: 1,
        borderColor: "divider",
        typography: "caption",
        lineHeight: 1.4,
        color: "text.secondary",
      }}
    >
      <Box
        sx={{ display: "flex", alignItems: "center", gap: 0.75, minWidth: 0 }}
      >
        <ProviderIcon
          provider={call.provider}
          sx={{ fontSize: "1rem", flexShrink: 0 }}
        />
        <Box
          component="span"
          role="heading"
          aria-level={2}
          sx={{
            fontSize: "0.8125rem",
            fontWeight: 700,
            color: "text.primary",
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
            minWidth: 0,
            flexShrink: 1,
          }}
        >
          {displayTitle(call)}
        </Box>
      </Box>
      <Box
        data-call-header-metadata
        sx={{
          display: "flex",
          alignItems: "center",
          flexWrap: "nowrap",
          columnGap: 0.75,
          rowGap: 0.5,
          minWidth: 0,
          overflowX: "auto",
          scrollbarWidth: "none",
          "& > *": { flexShrink: 0 },
        }}
      >
        <CallAccountBadge call={call} />
        {/* A finished review's findings already say it completed. */}
        {(call.state !== "completed" || !outcome) && (
          <Badge
            label={callStateLabel(call)}
            color={
              tone === "success"
                ? "success"
                : tone === "error"
                  ? "error"
                  : tone === "active"
                    ? "primary"
                    : "default"
            }
          />
        )}
        {outcome && (
          <Badge label={outcome} color={attention ? "warning" : "success"} />
        )}
        {configuration && (
          <Box component="span" data-call-header-configuration
            title={parameters.map(({ label, value }) => `${label}: ${value}`).join(" · ")}
            sx={{ display: "inline-flex", whiteSpace: "nowrap" }}>
            <Badge label={configuration} color="default" />
          </Box>
        )}
        <Box component="span" data-call-header-placement sx={{ display: "inline-flex", whiteSpace: "nowrap" }} title={placementLabel(call)}>
          <Badge label={placementLabel(call)} color="default" />
        </Box>
        <Box
          component="time"
          dateTime={new Date(call.started_at_ms ?? call.created_at_ms).toISOString()}
          data-call-header-age
          title={callTimesTitle(call)}
          sx={{ flexShrink: 0, whiteSpace: "nowrap", fontVariantNumeric: "tabular-nums" }}
        >
          {elapsedLabel(call.created_at_ms, end)} ·{" "}
          {callAgeLabel(call.started_at_ms ?? call.created_at_ms, now)}
        </Box>
      </Box>
    </Box>
  );
}

/** Which call the modal shows, and how deep: its transcript or its details. */
type CallRoute = { call: string; level: "transcript" | "details" };

/** The floating mobile footer island the newest message passes under. */
const MOBILE_FOOTER_CLEARANCE = "calc(76px + env(safe-area-inset-bottom, 0px))";

/** Level 2: the child's own conversation, headed by the call's state; its
 * details are one step further. */
function CallConversation({
  call,
  desktop,
  detailsOpen,
  onDetails,
}: {
  call: ManagedCallSummary;
  desktop: boolean;
  detailsOpen?: boolean;
  onDetails: () => void;
}): React.JSX.Element {
  const end = callActive(call.state) ? Date.now() : call.updated_at_ms;
  return (
    <Box
      data-call-conversation={call.call_id}
      // minWidth: an unbreakable transcript line must scroll in its block,
      // not widen the column past the modal beside the details panel.
      sx={{
        flex: 1,
        minWidth: 0,
        minHeight: 0,
        display: "flex",
        flexDirection: "column",
      }}
    >
      {
        /* Desktop: one line of state above the transcript. A phone shows the
          same line as the sheet's dense header (CallHeader). */
      }
      {desktop && <Stack
        direction="row"
        alignItems="center"
        spacing={1}
        sx={{
          px: desktop ? 0 : 2,
          pb: desktop ? 1 : 0.75,
          borderBottom: 1,
          borderColor: "divider",
          minWidth: 0,
        }}
      >
        <CallAccountBadge call={call} />
        <ProviderIcon
          provider={call.provider}
          sx={{ fontSize: "1rem", flexShrink: 0 }}
        />
        <Typography
          variant="caption"
          color="text.secondary"
          noWrap
          sx={{ flex: 1, minWidth: 0 }}
          title={callTimesTitle(call)}
        >
          {providerLabel(call.provider)} {call.provider_version ?? ""} ·{" "}
          {placementLabel(call)} · {startedLabel(call.created_at_ms)} ·{" "}
          {elapsedLabel(call.created_at_ms, end)}
        </Typography>
        {
          /* A verdict already says the review finished. */
        }
        {(call.state !== "completed" || !call.verdict) && (
          <StateChip call={call} desktop />
        )}
        {(call.verdict || call.finding_count !== undefined) && (
          <ButtonBase
            aria-label="Open review result"
            onClick={onDetails}
            sx={{ borderRadius: 999, minWidth: 0, flexShrink: 0 }}
          >
            <VerdictChip
              verdict={call.verdict ?? null}
              findings={call.finding_count}
              desktop
            />
          </ButtonBase>
        )}
        {desktop && (
          <Button
            size="small"
            variant={detailsOpen ? "contained" : "outlined"}
            disableElevation
            startIcon={<InfoOutlined fontSize="small" />}
            onClick={onDetails}
            aria-pressed={detailsOpen}
            sx={{ minHeight: 30, flexShrink: 0 }}
          >
            Details
          </Button>
        )}
      </Stack>}
      {call.error && (
        <Typography
          role="alert"
          variant="body2"
          color="error"
          sx={{ p: 2, overflowWrap: "anywhere" }}
        >
          {call.error.code}{call.error.detail ? `: ${call.error.detail}` : ""}
        </Typography>
      )}
      {callActive(call.state) || call.input_revision !== null
        ? (
          <CallTranscript
            sessionId={callConversationId(call)}
            provider={call.provider}
            settled={!callActive(call.state)}
            bottomInset={desktop ? "0px" : MOBILE_FOOTER_CLEARANCE}
            desktop={desktop}
          />
        )
        : (
          <Typography variant="body2" color="text.secondary" sx={{ p: 2 }}>
            The call ended before its input snapshot was ready. No conversation
            was started for this call.
          </Typography>
        )}
    </Box>
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
    <Stack spacing={desktop ? 1 : 1.5}>
      {groups.map(([group, members]) => (
        <Box key={group} role="group" aria-label={group || undefined}>
          {group && (
            <Typography
              variant="overline"
              color="text.secondary"
              component="div"
              noWrap
              sx={{
                px: 1,
                pb: 0.25,
                fontSize: "0.6875rem",
                fontWeight: 700,
                lineHeight: 1.6,
                letterSpacing: "0.04em",
              }}
            >
              {group}
              <Box component="span" sx={{ ml: 0.75, fontWeight: 500, color: "text.disabled" }}>
                {members.length}
              </Box>
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
function CallPreview({ call, title, now, onOpen }: {
  call: ManagedCallSummary;
  title: string;
  now: number;
  onOpen: () => void;
}): React.JSX.Element {
  const tap = useReliableTouchTap<HTMLButtonElement>(onOpen);
  const active = callActive(call.state);
  const tone = callTone(call.state);
  const end = active ? Date.now() : call.finished_at_ms ?? call.updated_at_ms;
  const time = callAgeLabel(call.started_at_ms ?? call.created_at_ms, now);
  const model = call.configuration?.model || call.preset;
  const reasoning = call.configuration?.reasoning_effort || call.configuration?.effort;
  const permission = call.configuration?.permission_mode || call.configuration?.sandbox_mode;
  return (
    <ButtonBase {...tap} data-call-preview={call.call_id}
      aria-label={`Open ${title}: ${callStateLabel(call)}`}
      title={`${callTimesTitle(call)} · ${placementLabel(call)} · ${callModelLabel(call)}`}
      sx={{
        flex: "0 0 auto", width: "max-content", minWidth: "14rem", maxWidth: "22rem", minHeight: 44,
        borderRadius: 1, textAlign: "left",
        justifyContent: "flex-start",
        "&:focus-visible": { outline: "2px solid", outlineColor: "primary.main", outlineOffset: -2 },
      }}
    >
      {/* Paint the permanent card surface inside the button: the touch theme
          clears ButtonBase hover/focus fills when a sheet returns focus. */}
      <Stack spacing={0.25} sx={{
        minWidth: 0, width: "100%", minHeight: 44, px: 1, py: 0.75,
        borderRadius: "inherit", bgcolor: "action.hover", justifyContent: "center",
      }}>
        <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0 }}>
          <ProviderIcon provider={call.provider} sx={{ fontSize: "1rem", flexShrink: 0 }} />
          <Typography data-call-preview-title variant="caption" noWrap sx={{ minWidth: 0, fontSize: "0.8125rem", fontWeight: 600, textTransform: "capitalize" }}>{title}</Typography>
          <CallAccountBadge call={call} />
        </Stack>
        <Box data-call-preview-configuration sx={{ display: "flex", alignItems: "center", gap: 0.5, minWidth: 0 }}>
          {(model || reasoning) ? (
            <>
              {model && <Typography variant="caption" color="text.secondary" noWrap title={call.configuration?.model ? `Model: ${model}` : `Preset: ${model}`}
                sx={{ minWidth: 0, fontSize: "0.75rem" }}>{call.configuration?.model ? model : `Preset: ${model}`}</Typography>}
              {reasoning && <Typography data-call-preview-reasoning variant="caption" color="text.secondary" noWrap title={`Reasoning: ${reasoning}`}
                sx={{ flexShrink: 0, fontSize: "0.75rem" }}>{model ? "· " : ""}{reasoning}</Typography>}
            </>
          ) : (
            <Typography variant="caption" color="text.secondary" noWrap sx={{ fontSize: "0.75rem" }}>Model not recorded</Typography>
          )}
          {permission && (
            <Typography variant="caption" color="text.secondary" noWrap title={`Permission: ${permission}`}
              sx={{ minWidth: 0, fontSize: "0.6875rem" }}>· {permission}</Typography>
          )}
        </Box>
        <Stack direction="row" spacing={0.5} alignItems="center" sx={{ minWidth: 0 }}>
          {active && <CircularProgress size="0.6875rem" />}
          <Typography variant="caption" noWrap sx={{
            color: tone === "error" ? "error.main" : tone === "success" ? "success.main" : active ? "primary.main" : "text.secondary",
            fontSize: "0.75rem", flexShrink: 0,
          }}>
            {callStateLabel(call)}{end - call.created_at_ms >= 1000 ? ` · ${elapsedLabel(call.created_at_ms, end)}` : ""}
          </Typography>
          <Typography component="time" dateTime={new Date(call.started_at_ms ?? call.created_at_ms).toISOString()}
            variant="caption" color="text.secondary" noWrap sx={{ flexShrink: 0, fontSize: "0.6875rem", fontVariantNumeric: "tabular-nums" }}>· {time}</Typography>
        </Stack>
      </Stack>
    </ButtonBase>
  );
}

function MobileCallRail({ calls, onOverview, onOpen, stale }: {
  calls: ManagedCallSummary[];
  onOverview: () => void;
  onOpen: (id: string) => void;
  stale: boolean;
}): React.JSX.Element {
  const tap = useReliableTouchTap<HTMLButtonElement>(onOverview);
  const now = useCallClock();
  const overview = callsOverview(calls);
  const titles = callPreviewTitles(calls);
  const ordered = [...calls].sort((a, b) => b.created_at_ms - a.created_at_ms);
  return (
    <Stack direction="row" alignItems="stretch" sx={{ minWidth: 0, gap: 0.5, py: 0.5 }}>
      <ButtonBase {...tap} aria-label="Open calls" sx={{ flexShrink: 0, px: 1, minHeight: 44 }}>
        <Stack alignItems="center" spacing={0.25}>
          <Typography variant="caption" sx={{ fontWeight: 600 }}>Calls</Typography>
          <Typography variant="caption" color={stale ? "warning.main" : "text.secondary"}>
            {stale ? "offline" : `${overview.total - overview.active}/${overview.total}`}
          </Typography>
        </Stack>
      </ButtonBase>
      <Box data-call-preview-rail data-mobile-drawer-ignore role="group" aria-label="Call previews"
        sx={{ display: "flex", minWidth: 0, flex: 1, gap: 0.5, overflowX: "auto", overscrollBehaviorX: "contain", scrollbarWidth: "none", "&::-webkit-scrollbar": { display: "none" } }}>
        {ordered.map((call) => <CallPreview key={call.call_id} call={call} title={titles.get(call.call_id)!} now={now} onOpen={() => onOpen(call.call_id)} />)}
      </Box>
    </Stack>
  );
}

/** A one-second clock for the few live elapsed labels on screen; the shared
 * call clock ticks only every 30 seconds. */
function useSecondClock(live: boolean): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!live) return undefined;
    const timer = globalThis.setInterval(() => {
      if (document.visibilityState !== "hidden") setNow(Date.now());
    }, 1000);
    return () => globalThis.clearInterval(timer);
  }, [live]);
  return now;
}

function stateBadgeColor(
  call: ManagedCallSummary,
): "success" | "error" | "primary" | "default" {
  const tone = callTone(call.state);
  return tone === "success"
    ? "success"
    : tone === "error"
    ? "error"
    : tone === "active"
    ? "primary"
    : "default";
}

/** Desktop: the Calls modal's left rail. Switching calls keeps the modal, so
 * a review round reads like tabs instead of close-and-reopen. */
function DesktopCallRail({ calls, selected, onSelect }: {
  calls: readonly ManagedCallSummary[];
  selected: string;
  onSelect: (call: string) => void;
}): React.JSX.Element {
  const now = useSecondClock(calls.some((call) => callActive(call.state)));
  return (
    <Box
      component="nav"
      aria-label="Calls"
      data-call-rail
      sx={{
        width: "clamp(220px, 18vw, 300px)",
        flexShrink: 0,
        overflowY: "auto",
        overscrollBehavior: "contain",
        ...desktopScrollbarSx,
        borderRight: 1,
        borderColor: "divider",
        pr: 1,
        display: "flex",
        flexDirection: "column",
        gap: 0.25,
      }}
    >
      {calls.map((call) => {
        const active = callActive(call.state);
        const current = call.call_id === selected;
        const end = active ? now : call.finished_at_ms ?? call.updated_at_ms;
        return (
          <ButtonBase
            key={call.call_id}
            data-call-rail-item={call.call_id}
            aria-current={current ? "true" : undefined}
            aria-label={`${callTitle(call)}: ${callStateLabel(call)}`}
            onClick={(): void => onSelect(call.call_id)}
            sx={{
              display: "flex",
              flexDirection: "column",
              alignItems: "stretch",
              gap: 0.375,
              px: 1,
              py: 0.75,
              borderRadius: 1,
              textAlign: "left",
              bgcolor: current ? "action.selected" : "transparent",
              "&:hover": { bgcolor: current ? "action.selected" : "action.hover" },
              "&:focus-visible": {
                outline: "2px solid",
                outlineColor: "primary.main",
                outlineOffset: -2,
              },
            }}
          >
            <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0 }}>
              <ProviderIcon provider={call.provider} sx={{ fontSize: "1rem", flexShrink: 0 }} />
              <Typography
                variant="caption"
                noWrap
                sx={{
                  flex: 1,
                  minWidth: 0,
                  fontSize: "0.8125rem",
                  fontWeight: current ? 700 : 600,
                  textTransform: "capitalize",
                }}
              >
                {callTitle(call)}
              </Typography>
            </Stack>
            <Stack direction="row" spacing={0.5} alignItems="center" sx={{ minWidth: 0, pl: "1.375rem" }}>
              <CallOutcomeBadge call={call} />
              <Typography
                variant="caption"
                color="text.secondary"
                noWrap
                sx={{ fontVariantNumeric: "tabular-nums", minWidth: 0 }}
              >
                {elapsedLabel(call.created_at_ms, end)}
              </Typography>
            </Stack>
          </ButtonBase>
        );
      })}
    </Box>
  );
}

/** Desktop: a running call in the collapsed Calls rail. One click (or `␣G`
 * for the newest) opens its live transcript without expanding the list. */
function ActiveCallChip({ call, onOpen }: {
  call: ManagedCallSummary;
  onOpen: () => void;
}): React.JSX.Element {
  const now = useSecondClock(true);
  return (
    <ButtonBase
      data-call-chip={call.call_id}
      aria-label={`Open ${callTitle(call)}: ${callStateLabel(call)}`}
      title={`${callModelLabel(call)} · ${placementLabel(call)}`}
      onClick={(event): void => {
        event.stopPropagation();
        onOpen();
      }}
      sx={(theme) => ({
        flexShrink: 1,
        minWidth: 0,
        maxWidth: "18rem",
        height: desktopSize(22),
        px: 0.75,
        gap: 0.5,
        borderRadius: 999,
        color: "primary.main",
        bgcolor: alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.22 : 0.12),
        "&:hover, &:focus-visible": {
          bgcolor: alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.32 : 0.2),
        },
        "&:focus-visible": { outline: "2px solid", outlineColor: "primary.main" },
      })}
    >
      <CircularProgress size={desktopSize(10)} thickness={5} color="inherit" sx={{ flexShrink: 0 }} />
      <ProviderIcon provider={call.provider} sx={{ fontSize: "0.875rem", flexShrink: 0 }} />
      <Box
        component="span"
        sx={{
          minWidth: 0,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
          fontSize: "0.75rem",
          fontWeight: 600,
          textTransform: "capitalize",
        }}
      >
        {callTitle(call)}
      </Box>
      <Box
        component="span"
        sx={{
          flexShrink: 0,
          fontSize: "0.6875rem",
          fontVariantNumeric: "tabular-nums",
          opacity: 0.8,
        }}
      >
        {elapsedLabel(call.started_at_ms ?? call.created_at_ms, now)}
      </Box>
    </ButtonBase>
  );
}

/** At most this many running calls get a chip; the rest are counted. */
const ACTIVE_CHIP_LIMIT = 3;

export const ManagedCallsDock = memo(function ManagedCallsDock({
  sessionId,
  desktop,
  shortcut,
}: {
  sessionId: string;
  desktop: boolean;
  shortcut?: ReactNode;
}): React.JSX.Element | null {
  const { calls, observedAt, error, refresh } = useManagedCalls(sessionId);
  const expanded = useStore(composerStackExpandedStore()) === "calls";
  const [route, setRoute] = useState<CallRoute | null>(null);
  const [mobileOpen, setMobileOpen] = useState(false);
  const [desktopOpen, setDesktopOpen] = useState(false);
  const [desktopDetails, setDesktopDetails] = useState(true);
  const toggle = (): void => {
    if (desktop) toggleComposerStackPanel("calls");
    else {
      // Closing the page leaves it; every open starts at the list. Reset here
      // rather than on close so the closing sheet keeps its content.
      setRoute(null);
      setMobileOpen(true);
    }
  };
  const toggleTap = useReliableTouchTap<HTMLButtonElement>(
    toggle,
    swallowStrayDisclosureClick,
  );
  // An open Mobile page keeps its sheet while its last call is cleaned up;
  // Desktop never shows an empty "0/0" rail.
  if (calls.length === 0 && (observedAt === null || !mobileOpen)) return null;
  const overview = callsOverview(calls);
  const routed = route
    ? calls.find((call) => call.call_id === route.call) ?? null
    : null;
  const level = routed ? route!.level : null;
  const openCall = (call: string): void => {
    setRoute({ call, level: "transcript" });
    if (desktop) setDesktopOpen(true);
    else setMobileOpen(true);
  };
  const back = (): void =>
    setRoute(
      route && route.level === "details"
        ? { call: route.call, level: "transcript" }
        : null,
    );
  // `[` / `]` step through calls in the rail's order.
  const stepCall = (delta: number): void => {
    if (!routed) return;
    const index = calls.findIndex((call) => call.call_id === routed.call_id);
    const next = calls[index + delta];
    if (next) setRoute({ call: next.call_id, level: "transcript" });
  };
  const desktopKeys = (event: KeyboardEvent): void => {
    const target = event.target as HTMLElement;
    if (
      event.metaKey || event.ctrlKey || event.altKey ||
      // React bubbles portal events: a Tool inspector opened from the child
      // transcript owns its own keys.
      !event.currentTarget.contains(target) ||
      target.closest("input, textarea, [contenteditable='true']")
    ) return;
    if (event.key === "i") {
      event.preventDefault();
      setDesktopDetails((open) => !open);
    } else if (event.key === "o" && routed) {
      event.preventDefault();
      openChild(callConversationId(routed));
    } else if (event.key === "[" || event.key === "]") {
      event.preventDefault();
      stepCall(event.key === "[" ? -1 : 1);
    }
  };
  const stale = error !== null && observedAt !== null;
  function openChild(child: string): void {
    setMobileOpen(false);
    setDesktopOpen(false);
    setActiveSessionId(child);
  }
  const summary = overview.active > 0
    ? `${overview.activeTitle ?? "Working"}${
      overview.active > 1 ? ` +${overview.active - 1}` : ""
    }`
    : overview.failed > 0
    ? `${overview.failed} failed`
    : "All calls finished";
  // Newest first: the chip the Calls shortcut opens is the call that started last.
  const activeCalls = calls
    .filter((call) => callActive(call.state))
    .sort((a, b) => b.created_at_ms - a.created_at_ms);
  return (
    <Box
      data-desktop-calls-surface={desktop ? "true" : undefined}
      sx={{
        // Desktop adopts the same borderless disclosure rail as Queue and
        // Drafts. Mobile keeps its established card and touch geometry.
        ...(desktop
          ? {
            border: 0,
            borderBottom: 1,
            borderColor: "divider",
            borderRadius: 0,
            bgcolor: "transparent",
            transition: "background-color 120ms ease, border-color 120ms ease",
            "&:focus-within": { borderColor: "primary.main" },
          }
          : { ...mobileComposerPanelFrameSx, border: 0, bgcolor: "transparent" }),
        mb: desktop ? 0.5 : 0,
        // Own the text color and typography: rows and details are rendered in
        // ButtonBase/Sheet surfaces that would otherwise inherit the page's.
        color: "text.primary",
        typography: "body2",
        overflow: "hidden",
      }}
    >
      {!desktop
        ? <MobileCallRail calls={calls} onOverview={toggle} onOpen={openCall} stale={stale} />
        : (
          // The rail is a toggle plus, while calls run, one chip per call. The
          // chips are buttons of their own, so the toggle cannot wrap them;
          // the rest of the row still toggles on click.
          <Stack
            direction="row"
            alignItems="center"
            onClick={(event): void => {
              if (!(event.target as HTMLElement).closest("button")) toggle();
            }}
            sx={{
              width: "100%",
              minWidth: 0,
              pr: 1,
              minHeight: desktopSize(32),
              gap: "0.375rem",
              cursor: "pointer",
              "@media (pointer: coarse)": {
                minHeight: mobileComposerPanelHeaderMinHeight,
              },
            }}
          >
            <ButtonBase
              {...toggleTap}
              aria-label={expanded ? "Collapse calls" : "Expand calls"}
              aria-expanded={expanded}
              disableRipple
              sx={{
                flexShrink: 0,
                alignSelf: "stretch",
                justifyContent: "flex-start",
                gap: "0.375rem",
                pl: 0.5,
                pr: 0.5,
                borderRadius: 1,
                touchAction: "manipulation",
                "&:active, &:focus-visible": { bgcolor: "action.hover" },
              }}
            >
              {/* With the 0.375rem gap this is the 28px (scaled) chevron
                  column the Queue/Drafts header uses: one shared left edge. */}
              <Box
                sx={{
                  width: desktopSize(22),
                  display: "inline-flex",
                  justifyContent: "center",
                  flexShrink: 0,
                }}
              >
                {expanded
                  ? <ExpandMore fontSize="small" />
                  : <ChevronRight fontSize="small" />}
              </Box>
              <Typography variant="caption" sx={{ fontWeight: 600 }}>
                Calls
              </Typography>
            </ButtonBase>
            {activeCalls.length > 0
              ? (
                <Box
                  data-calls-active-chips
                  sx={{
                    flex: 1,
                    minWidth: 0,
                    display: "flex",
                    alignItems: "center",
                    gap: 0.5,
                    overflow: "hidden",
                  }}
                >
                  {activeCalls.slice(0, ACTIVE_CHIP_LIMIT).map((call) => (
                    <ActiveCallChip
                      key={call.call_id}
                      call={call}
                      onOpen={(): void => openCall(call.call_id)}
                    />
                  ))}
                  {activeCalls.length > ACTIVE_CHIP_LIMIT && (
                    <Typography
                      variant="caption"
                      color="text.secondary"
                      sx={{ flexShrink: 0, fontWeight: 600 }}
                    >
                      +{activeCalls.length - ACTIVE_CHIP_LIMIT}
                    </Typography>
                  )}
                </Box>
              )
              : (
                <Typography
                  variant="caption"
                  noWrap
                  sx={{
                    flex: 1,
                    minWidth: 0,
                    color: overview.failed > 0 ? "error.main" : "text.secondary",
                    textTransform: "capitalize",
                  }}
                >
                  {summary}
                </Typography>
              )}
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
                sx={{ fontWeight: 600, flexShrink: 0 }}
              >
                {overview.findings} finding{overview.findings === 1 ? "" : "s"}
              </Typography>
            )}
            {shortcut}
            <Typography
              variant="caption"
              color="text.secondary"
              sx={{ fontWeight: 600, fontVariantNumeric: "tabular-nums", flexShrink: 0 }}
            >
              {overview.total - overview.active}/{overview.total}
            </Typography>
          </Stack>
        )}
      {desktop && (
        <Collapse
        in={expanded}
        timeout={COMPOSER_STACK_DISCLOSURE_TIMEOUT}
        sx={composerStackDisclosureSx(expanded)}
      >
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
              selected={desktopOpen && routed ? routed.call_id : null}
              desktop
              onSelect={openCall}
            />
            <CallCleanup key={sessionId} parent={sessionId} calls={calls} onChange={refresh} />
          </Stack>
        </Collapse>
      )}
      {desktop && (
        <Sheet
          open={desktopOpen && routed !== null}
          onClose={(): void =>
            setDesktopOpen(false)}
          title={routed ? displayTitle(routed) : "Call"}
          desktopMaxWidth={1240}
          desktopFill
        >
          {routed && (
            <Box
              data-desktop-call-modal={routed.call_id}
              data-desktop-keys="own"
              onKeyDown={desktopKeys}
              sx={{
                flex: 1,
                display: "flex",
                flexDirection: "column",
                minHeight: 0,
                color: "text.primary",
                typography: "body2",
              }}
            >
              <Stack
                direction="row"
                alignItems="center"
                spacing={1}
                sx={{ pb: 1, minWidth: 0 }}
              >
                <Typography
                  variant="subtitle1"
                  noWrap
                  sx={{
                    flex: 1,
                    minWidth: 0,
                    fontWeight: 700,
                    textTransform: "capitalize",
                  }}
                >
                  {callTitle(routed)}
                </Typography>
                <HintTooltip title="Close" shortcut="Escape">
                  <IconButton
                    aria-label="Close call"
                    size="small"
                    onClick={(): void => setDesktopOpen(false)}
                  >
                    <Close fontSize="small" />
                  </IconButton>
                </HintTooltip>
              </Stack>
              <Stack
                direction="row"
                spacing={2}
                sx={{ flex: 1, minHeight: 0 }}
              >
                {calls.length > 1 && (
                  <DesktopCallRail
                    calls={calls}
                    selected={routed.call_id}
                    onSelect={(call): void =>
                      setRoute({ call, level: "transcript" })}
                  />
                )}
                <CallConversation
                  call={routed}
                  desktop
                  detailsOpen={desktopDetails}
                  onDetails={(): void => setDesktopDetails((open) => !open)}
                />
                {desktopDetails && (
                  <Box
                    data-call-details-panel
                    sx={{
                      width: "clamp(320px, 24vw, 440px)",
                      flexShrink: 0,
                      overflowY: "auto",
                      overscrollBehavior: "contain",
                      ...desktopScrollbarSx,
                      borderLeft: 1,
                      borderColor: "divider",
                      pl: 2,
                      pb: 2,
                    }}
                  >
                    <CallDetailView
                      parent={sessionId}
                      call={routed}
                      desktop
                      onOpenChild={openChild}
                    />
                  </Box>
                )}
              </Stack>
              <Box sx={{ mx: -3, mt: 1, borderTop: 1, borderColor: "divider" }}>
                <DesktopShortcutBar
                  groups={[
                    {
                      label: "Call",
                      slots: [{
                        shortcut: "[/]",
                        label: "Previous / next",
                        availability: calls.length > 1
                          ? "available"
                          : "inactive",
                      }],
                    },
                    {
                      slots: [
                        {
                          shortcut: "I",
                          label: "Details",
                          availability: desktopDetails ? "active" : "available",
                        },
                        { shortcut: "O", label: "Open as session" },
                        { shortcut: "Esc", label: "Close" },
                      ],
                    },
                  ]}
                />
              </Box>
            </Box>
          )}
        </Sheet>
      )}
      {!desktop && (
        <Sheet
          open={mobileOpen}
          onClose={(): void => setMobileOpen(false)}
          title={!routed
            ? "Calls"
            : level === "details"
            ? "Details"
            // The transcript level names itself in its dense header.
            : displayTitle(routed)}
          denseHeader={routed && level === "transcript"
            ? <CallHeader call={routed} />
            : undefined}
          onBack={routed ? back : undefined}
          islandActions={routed && level === "transcript"
            ? [
              {
                label: "Scroll to start",
                icon: <VerticalAlignTop fontSize="small" />,
                onActivate: (): void =>
                  requestScrollToTop(callConversationId(routed)),
              },
              {
                label: "Details",
                icon: <InfoOutlined fontSize="small" />,
                onActivate: (): void =>
                  setRoute({ call: routed.call_id, level: "details" }),
              },
              {
                // There is no Follow toggle here; this is the way back to the
                // newest message after reading earlier ones.
                label: "Scroll to latest",
                icon: <VerticalAlignBottom fontSize="small" />,
                onActivate: (): void =>
                  requestStickToBottom(callConversationId(routed)),
              },
            ]
            : undefined}
          fill={level === "transcript"}
          forceSheet
          cover
          portal
        >
          {routed && level === "transcript"
            ? (
              <CallConversation
                call={routed}
                desktop={false}
                onDetails={(): void =>
                  setRoute({ call: routed.call_id, level: "details" })}
              />
            )
            : (
              <Box sx={{ pb: 2, color: "text.primary", typography: "body2" }}>
                {routed
                  ? (
                    <CallDetailView
                      parent={sessionId}
                      call={routed}
                      desktop={false}
                      onOpenChild={openChild}
                    />
                  )
                  : (
                    <Stack spacing={1}>
                      <CallCleanup key={sessionId} parent={sessionId} calls={calls} onChange={refresh} />
                      <CallList calls={calls} selected={null} desktop={false} onSelect={openCall} />
                    </Stack>
                  )}
              </Box>
            )}
        </Sheet>
      )}
    </Box>
  );
});

/** Shown instead of the Prompt in a managed child: one controller owns it.
 * Details opens the same call details the parent's Calls modal shows. */
export function ManagedChildNotice(
  { parent, child }: { parent: string; child: string },
): React.JSX.Element {
  const { calls } = useManagedCalls(parent);
  const theme = useTheme();
  const desktop = useMediaQuery(theme.breakpoints.up("md"));
  const [detailsOpen, setDetailsOpen] = useState(false);
  // The newest call that ran this child (a continued child has several).
  const call = calls
    .filter((candidate) => candidate.child_session_id === child)
    .sort((left, right) => right.created_at_ms - left.created_at_ms)[0];
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
        {call && (
          <Button
            size="small"
            variant="outlined"
            startIcon={<InfoOutlined fontSize="small" />}
            onClick={(): void => setDetailsOpen(true)}
            sx={{ minHeight: 44 }}
          >
            Details
          </Button>
        )}
        <Button
          size="small"
          variant="outlined"
          onClick={(): void => setActiveSessionId(parent)}
          sx={{ minHeight: 44 }}
        >
          Open parent
        </Button>
      </Stack>
      {call && (
        <Sheet
          open={detailsOpen}
          onClose={(): void => setDetailsOpen(false)}
          title="Details"
          {...(desktop ? { desktopMaxWidth: 560 } : { forceSheet: true })}
          cover
          portal
        >
          <Box sx={{ pb: 2, color: "text.primary", typography: "body2" }}>
            <CallDetailView
              parent={parent}
              call={call}
              desktop={desktop}
            />
          </Box>
        </Sheet>
      )}
    </Box>
  );
}
