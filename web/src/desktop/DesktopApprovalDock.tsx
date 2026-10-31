import { type ReactNode, useLayoutEffect } from "react";
import {
  alpha,
  Box,
  Button,
  CircularProgress,
  Stack,
  Typography,
} from "@mui/material";
import AccountTree from "@mui/icons-material/AccountTree";
import HelpOutlineRounded from "@mui/icons-material/HelpOutlineRounded";
import WarningAmberRounded from "@mui/icons-material/WarningAmberRounded";
import { agentLabel } from "../agentTools";
import { claimApprovalDock } from "../approvalDockStore";
import {
  type CallApproval,
  type CallApprovalDecision,
  callApprovalReason,
  callApprovalTitle,
  useCallApproval,
  useCallApprovalAnswer,
} from "../callApproval";
import {
  composerTimelineSlice,
  sameComposerTimelineSlice,
} from "../composerTimelineSlice";
import type { PendingPermission } from "../derive";
import { usePermissionResponse } from "../permissionResponse";
import {
  elicitationChoiceOf,
  ElicitationFields,
  useElicitationForm,
} from "../ElicitationFields";
import { useStoreSelector } from "../store";
import { desktopSize } from "../surface/desktopSize";
import {
  type DesktopApprovalChoice,
  DesktopApprovalCommands,
  DesktopApprovalKeycap,
} from "./DesktopApprovalCommands";

const MONOSPACE = "ui-monospace, SFMono-Regular, Menlo, monospace";

type Tone = "warning" | "primary";

interface DockChoice extends DesktopApprovalChoice {
  id: string;
  /** A refusal is outlined in the error colour, never the primary fill. */
  reject: boolean;
  primary: boolean;
  /** An incomplete form's Submit stays visible but cannot answer yet. */
  disabled?: boolean;
}

/**
 * The Conversation pane's approval dock (FOCUS.md "Approvals"). A blocked
 * agent's question is answered where its output is read: docked under the
 * transcript at the pane's full width, outside the scroll area, so it neither
 * scrolls away nor moves while output streams above it. It claims the session
 * so the Composer does not also draw its overlay.
 */
export function DesktopApprovalDock({
  sessionId,
}: {
  sessionId: string;
}): React.JSX.Element | null {
  useLayoutEffect(() => claimApprovalDock(sessionId), [sessionId]);
  const timeline = useStoreSelector(
    (snapshot) => composerTimelineSlice(snapshot.timelines.get(sessionId)),
    sameComposerTimelineSlice,
  );
  const callApproval = useCallApproval(sessionId);
  // A tool permission outranks an agent call, as in the Composer.
  if (timeline.pendingPermission) {
    return (
      <DesktopPermissionDock
        key={timeline.pendingPermission.requestId}
        item={timeline.pendingPermission}
        waiting={timeline.pendingPermissionCount}
        sessionId={sessionId}
      />
    );
  }
  if (callApproval) {
    return <CallApprovalDock approval={callApproval} sessionId={sessionId} />;
  }
  return null;
}

/** One request's dock; keyed by request so each starts unanswered. */
export function DesktopPermissionDock({
  item,
  waiting,
  sessionId,
}: {
  item: PendingPermission;
  waiting: number;
  sessionId: string;
}): React.JSX.Element {
  const response = usePermissionResponse(sessionId, item.requestId);
  const question = item.requestKind === "question";
  const form = useElicitationForm(item.requestId, item.elicitation, response.respond);
  const firstAllow = item.options.findIndex((option) =>
    !option.kind.toLowerCase().startsWith("reject")
  );
  const choices: DockChoice[] = item.elicitation
    ? form.choices.map((choice) => ({
      id: choice.id,
      label: choice.label,
      reject: choice.reject,
      primary: !choice.reject,
      disabled: choice.disabled,
      run: choice.run,
    }))
    : item.options.map((option, index) => ({
      id: option.optionId,
      label: option.name,
      reject: option.kind.toLowerCase().startsWith("reject"),
      primary: index === firstAllow,
      run: () => response.respond(option.optionId),
    }));
  return (
    <ApprovalDock
      kind="permission"
      tone={question ? "primary" : "warning"}
      icon={question
        ? <HelpOutlineRounded sx={{ fontSize: desktopSize(16) }} />
        : <WarningAmberRounded sx={{ fontSize: desktopSize(16) }} />}
      title={question ? "Answer needed" : "Permission required"}
      detail={[
        item.subagent ? `Asked by subagent ${item.subagent}` : null,
        waiting > 1 ? `1 of ${String(waiting)} waiting` : null,
      ].filter((part) => part !== null).join(" · ") || null}
      choices={choices}
      busyId={response.busy
        ? item.elicitation ? elicitationChoiceOf(response.optionId) : response.optionId
        : null}
      notice={response.notice}
      noticeTone={response.phase === "unsent" ? "error" : "warning"}
    >
      {item.elicitation
        ? (
          <ElicitationFields
            elicitation={item.elicitation}
            values={form.values}
            setValue={form.setValue}
            disabled={response.busy}
            seenKey={`${sessionId}:${item.seq}:${item.requestId}`}
          />
        )
        : (
      <Box
        component="pre"
        data-desktop-approval-subject
        sx={{
          m: 0,
          px: 1,
          py: 0.75,
          maxHeight: "min(30vh, 260px)",
          overflow: "auto",
          borderRadius: 1,
          bgcolor: "action.hover",
          // A command is read character by character before it is allowed.
          fontFamily: question ? "inherit" : MONOSPACE,
          fontSize: desktopSize(12.5),
          lineHeight: 1.5,
          whiteSpace: "pre-wrap",
          overflowWrap: "anywhere",
          userSelect: "text",
        }}
      >
        {item.title}
      </Box>
        )}
    </ApprovalDock>
  );
}

const CALL_DECISIONS: readonly CallApprovalDecision[] = [
  "once",
  "session",
  "decline",
];

function CallApprovalDock({
  approval,
  sessionId,
}: {
  approval: CallApproval;
  sessionId: string;
}): React.JSX.Element {
  const { busy, failure, answer } = useCallApprovalAnswer(sessionId, approval);
  const labels: Record<CallApprovalDecision, string> = {
    once: approval.requests === 1 ? "Allow once" : "Allow these calls",
    session: "Allow for this session",
    decline: "Decline",
  };
  const choices: DockChoice[] = CALL_DECISIONS.map((decision) => ({
    id: decision,
    label: labels[decision],
    reject: decision === "decline",
    primary: decision === "once",
    run: () => answer(decision),
  }));
  const shown = approval.items.slice(0, 3);
  const more = approval.requests - shown.length;
  return (
    <ApprovalDock
      kind="call"
      tone="primary"
      icon={<AccountTree sx={{ fontSize: desktopSize(16) }} />}
      title={callApprovalTitle(approval)}
      detail={null}
      choices={choices}
      busyId={busy}
      notice={failure}
      noticeTone="error"
    >
      <Typography variant="caption" sx={{ color: "text.secondary" }}>
        {callApprovalReason(approval)}{" "}
        Calls start read-only reviewers and use tokens.
      </Typography>
      <Stack
        component="ul"
        spacing={0.25}
        sx={{
          m: 0,
          mt: 0.5,
          px: 1,
          py: 0.5,
          listStyle: "none",
          borderRadius: 1,
          bgcolor: "action.hover",
          fontSize: desktopSize(12.5),
        }}
      >
        {shown.map((item, index) => (
          <Box
            component="li"
            key={index}
            sx={{ display: "flex", gap: 1, minWidth: 0 }}
          >
            <Box component="span" sx={{ fontWeight: 650, flexShrink: 0 }}>
              {agentLabel(item.agent)}
            </Box>
            <Box
              component="span"
              sx={{
                color: "text.secondary",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {item.purpose.replace("_", " ")} · {item.summary}
            </Box>
          </Box>
        ))}
        {more > 0 && (
          <Box component="li" sx={{ color: "text.secondary" }}>
            +{more} more
          </Box>
        )}
      </Stack>
    </ApprovalDock>
  );
}

function ApprovalDock({
  kind,
  tone,
  icon,
  title,
  detail,
  choices,
  busyId,
  notice,
  noticeTone,
  children,
}: {
  kind: "permission" | "call";
  tone: Tone;
  icon: ReactNode;
  title: string;
  detail: string | null;
  choices: readonly DockChoice[];
  /** The choice whose answer is in flight; every choice waits on it. */
  busyId: string | null;
  notice: string | null;
  noticeTone: "error" | "warning";
  children: ReactNode;
}): React.JSX.Element {
  const busy = busyId !== null;
  return (
    <Box
      role="group"
      aria-label={title}
      data-desktop-approval-dock={kind}
      sx={(theme) => ({
        flexShrink: 0,
        minWidth: 0,
        px: 1.5,
        py: 1,
        display: "flex",
        flexDirection: "column",
        gap: 0.75,
        borderTop: 1,
        borderColor: alpha(theme.palette[tone].main, 0.45),
        bgcolor: alpha(
          theme.palette[tone].main,
          theme.palette.mode === "dark" ? 0.12 : 0.07,
        ),
      })}
    >
      <DesktopApprovalCommands choices={choices} available={!busy} />
      <Stack
        direction="row"
        alignItems="center"
        spacing={0.75}
        sx={{ color: `${tone}.main`, minWidth: 0 }}
      >
        {icon}
        <Typography
          variant="caption"
          noWrap
          sx={{ fontWeight: 700, lineHeight: 1.2, minWidth: 0 }}
        >
          {title}
        </Typography>
        {detail && (
          <Typography
            variant="caption"
            noWrap
            data-desktop-approval-waiting
            sx={{ color: "text.secondary", lineHeight: 1.2, flexShrink: 0 }}
          >
            {detail}
          </Typography>
        )}
      </Stack>
      <Box sx={{ minWidth: 0 }}>{children}</Box>
      <Stack
        direction="row"
        alignItems="center"
        useFlexGap
        sx={{ flexWrap: "wrap", gap: 0.75, minWidth: 0 }}
      >
        {choices.map((choice, index) => (
          <Button
            key={choice.id}
            size="small"
            disableElevation
            disabled={busy || choice.disabled === true}
            variant={choice.primary ? "contained" : "outlined"}
            color={choice.reject ? "error" : "primary"}
            data-desktop-approval-choice={String(index + 1)}
            onClick={choice.run}
            sx={{
              gap: 0.75,
              minHeight: desktopSize(28),
              py: 0.25,
              px: 1.25,
              textTransform: "none",
              fontWeight: 600,
              lineHeight: 1.3,
              textAlign: "left",
            }}
          >
            {busyId === choice.id && (
              <CircularProgress size={desktopSize(12)} thickness={5} color="inherit" />
            )}
            {choice.label}
            <DesktopApprovalKeycap index={index} available={!busy} />
          </Button>
        ))}
        {notice && (
          <Typography
            role="alert"
            variant="caption"
            sx={{ color: `${noticeTone}.main`, minWidth: 0 }}
          >
            {notice}
          </Typography>
        )}
      </Stack>
    </Box>
  );
}
