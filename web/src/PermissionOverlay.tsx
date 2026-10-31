import { desktopSize, glyphSize } from "./surface/desktopSize";
import type { ReactNode } from "react";
import {
  alpha,
  Box,
  Button,
  CircularProgress,
  Stack,
  Typography,
} from "@mui/material";
import WarningAmberRounded from "@mui/icons-material/WarningAmberRounded";
import HelpOutlineRounded from "@mui/icons-material/HelpOutlineRounded";
import type { PendingPermission } from "./derive";
import { usePermissionResponse } from "./permissionResponse";
import {
  elicitationChoiceOf,
  ElicitationFields,
  useElicitationForm,
} from "./ElicitationFields";
import { requestStickToBottom, useSticky } from "./stickyStore";
import { frostedPanel, frostedPill } from "./frostedGlass";
import { haptic } from "./haptic";
import {
  approvalButtonSx,
  approvalChoicesInRow,
  approvalProseSx,
} from "./approvalCardStyle";

// The pending tool-permission overlay — a sticky frosted surface floating just
// above the composer, in the SAME slot + material as TurnStatusOverlay (they
// never show at once: a permission request is mid-turn, and the turn-status pill
// hides while the agent is busy). This is the ACTIONABLE surface; the timeline
// keeps only a record marker (PermissionCard).
//
// A tool-approval is high-stakes and BLOCKS the agent, so it stays put:
//   - At the bottom (stuck-to-bottom): EXPANDED — the command verbatim + the
//     full-width option buttons (sentence-case, body-consistent type).
//   - Scrolled up (not stuck): AUTO-COLLAPSES to a compact pill so it never
//     covers history while you read; tapping it scrolls back to the bottom
//     (`requestStickToBottom`), which re-expands it.
//
// It participates in the Composer stack's normal flow, so the single outer
// geometry owner reserves its border box together with every other slot.
//
// Desktop draws this only while the Conversation pane, whose dock normally
// owns the answer, is folded (approvalDockStore). It then passes `desktop`:
// the numbered `␣1`…`␣9` slots, and no collapse, because a wheel tick on the
// way to a button must not swap the buttons for a pill.
export function PermissionOverlay({
  item,
  waiting = 1,
  sessionId,
  desktop,
}: {
  item: PendingPermission;
  /** Unresolved requests, this one included; they are answered in order. */
  waiting?: number;
  sessionId: string;
  desktop?: {
    /** Registers the numbered commands for the choices. */
    commands: (
      choices: { label: string; run: () => void }[],
      available: boolean,
    ) => ReactNode;
    keycap: (index: number, available: boolean) => ReactNode;
  };
}): React.JSX.Element {
  // `useSticky` is the existing "is the transcript pinned to the bottom" signal,
  // toggled the instant the user wheels/touches away — so it's exactly the
  // auto-collapse trigger (no new scroll listener on the fragile scroll model).
  const sticky = useSticky(sessionId);
  const expanded = sticky || desktop !== undefined;
  const isQuestion = item.requestKind === "question";
  const accent = isQuestion ? "primary.main" : "warning.main";
  const label = isQuestion ? "Answer needed" : "Permission required";
  const response = usePermissionResponse(sessionId, item.requestId);

  const respond = (optionId: string): void => {
    haptic();
    response.respond(optionId);
  };
  // An elicitation draws its own form and Submit/Decline (or Done/Decline);
  // a tool permission keeps the agent's options.
  const form = useElicitationForm(item.requestId, item.elicitation, respond);
  const choices = item.elicitation ? form.choices : item.options.map((option) => ({
    id: option.optionId,
    label: option.name,
    reject: option.kind.startsWith("reject"),
    disabled: false,
    run: () => respond(option.optionId),
  }));
  const busyChoice = item.elicitation
    ? elicitationChoiceOf(response.optionId)
    : response.optionId;

  return (
    <Box
      data-permission-overlay
      data-composer-stack-slot="status"
      sx={{
        position: "relative",
        display: "flex",
        justifyContent: "center",
        px: 2,
        width: "100%",
        minWidth: 0,
        boxSizing: "border-box",
        pointerEvents: "none",
        zIndex: 3,
      }}
    >
      {desktop?.commands(
        choices.map((choice) => ({ label: choice.label, run: choice.run })),
        !response.busy,
      )}
      {expanded ? (
        <Box
          sx={(t) => ({
            pointerEvents: "auto",
            width: "100%",
            maxWidth: 460,
            p: 1.5,
            borderRadius: 2.5,
            ...frostedPanel(t),
            // A warning tint over the frosted panel (the colour-coded "needs a
            // decision" state, matching the turn-status overlay's amber kinds).
            backgroundImage: `linear-gradient(0deg, ${
              alpha(
                isQuestion ? t.palette.primary.main : t.palette.warning.main,
                t.palette.mode === "dark" ? 0.16 : 0.12,
              )
            }, ${
              alpha(
                isQuestion ? t.palette.primary.main : t.palette.warning.main,
                t.palette.mode === "dark" ? 0.16 : 0.12,
              )
            })`,
          })}
        >
          <Stack
            direction="row"
            spacing={1}
            alignItems="center"
            sx={{ color: accent, mb: 1 }}
          >
            {isQuestion
              ? <HelpOutlineRounded fontSize="small" sx={{ flexShrink: 0 }} />
              : <WarningAmberRounded fontSize="small" sx={{ flexShrink: 0 }} />}
            <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
              {label}
            </Typography>
            {item.subagent && (
              <Typography
                variant="caption"
                data-permission-subagent
                noWrap
                sx={{ color: "text.secondary", minWidth: 0 }}
              >
                {item.subagent}
              </Typography>
            )}
            {waiting > 1 && (
              <Typography variant="caption" sx={{ color: "text.secondary" }}>
                1 of {waiting} waiting
              </Typography>
            )}
          </Stack>
          {item.elicitation
            ? (
              <Box sx={{ mb: 1.5 }}>
                <ElicitationFields
                  elicitation={item.elicitation}
                  values={form.values}
                  setValue={form.setValue}
                  disabled={response.busy}
                />
              </Box>
            )
            : (
          <Box
            component="pre"
            sx={{
              m: 0,
              mb: 1.5,
              px: 1,
              py: 0.75,
              borderRadius: 1,
              bgcolor: "action.hover",
              // A tool request names a command; a plain question is prose.
              ...(isQuestion
                ? approvalProseSx
                : { fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace" }),
              fontSize: isQuestion ? "0.875rem" : "0.8125rem",
              whiteSpace: "pre-wrap",
              // A command fills each line; a long path is one unbreakable "word".
              wordBreak: isQuestion ? "break-word" : "break-all",
            }}
          >
            {item.title}
          </Box>
            )}
          <Stack
            direction={approvalChoicesInRow(choices.map((choice) => choice.label))
              ? "row"
              : "column"}
            spacing={1}
            sx={{ "& > .MuiButton-root": { flex: 1, minWidth: 0 } }}
          >
            {choices.map((choice, index) => {
              const reject = choice.reject;
              // Only the first (narrowest) approval is the filled default;
              // a broader grant such as "always allow" stays secondary.
              const primary = !reject &&
                choices.findIndex((other) => !other.reject) === index;
              return (
                <Button
                  key={choice.id}
                  fullWidth
                  disableElevation
                  disabled={response.busy || choice.disabled}
                  variant={primary ? "contained" : "outlined"}
                  color={reject ? "error" : "primary"}
                  startIcon={response.busy && busyChoice === choice.id
                    ? <CircularProgress size={desktopSize(14)} thickness={5} color="inherit" />
                    : undefined}
                  endIcon={desktop?.keycap(index, !response.busy)}
                  onClick={choice.run}
                  sx={approvalButtonSx}
                >
                  {choice.label}
                </Button>
              );
            })}
          </Stack>
          {response.notice && (
            <Typography
              role="alert"
              variant="body2"
              sx={{
                mt: 1,
                color: response.phase === "unsent" ? "error.main" : "warning.main",
              }}
            >
              {response.notice}
            </Typography>
          )}
        </Box>
      ) : (
        <Stack
          role="status"
          direction="row"
          alignItems="center"
          spacing={1}
          onClick={(): void => {
            haptic();
            requestStickToBottom(sessionId);
          }}
          sx={(t) => ({
            pointerEvents: "auto",
            cursor: "pointer",
            maxWidth: "100%",
            px: 2,
            py: 0.5,
            minHeight: 36,
            borderRadius: 999,
            userSelect: "none",
            WebkitUserSelect: "none",
            ...frostedPill(t, isQuestion ? t.palette.primary.main : t.palette.warning.main),
          })}
        >
          {isQuestion
            ? <HelpOutlineRounded sx={{ fontSize: glyphSize(18), color: accent, flexShrink: 0 }} />
            : (
              <WarningAmberRounded
                sx={{ fontSize: glyphSize(18), color: accent, flexShrink: 0 }}
              />
            )}
          <Typography
            variant="body2"
            sx={{ fontWeight: 600, color: accent, whiteSpace: "nowrap" }}
          >
            {label}
          </Typography>
        </Stack>
      )}
    </Box>
  );
}
