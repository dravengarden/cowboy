import { type ReactNode, useMemo } from "react";
import { alpha, Box, Button, Stack, Typography } from "@mui/material";
import WarningAmberRounded from "@mui/icons-material/WarningAmberRounded";
import HelpOutlineRounded from "@mui/icons-material/HelpOutlineRounded";
import type { PendingPermission } from "./derive";
import { send } from "./store";
import { requestStickToBottom, useSticky } from "./stickyStore";
import { frostedPanel, frostedPill } from "./frostedGlass";
import { haptic } from "./haptic";

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
export function PermissionOverlay({
  item,
  sessionId,
  shortcutForAction,
}: {
  item: PendingPermission;
  sessionId: string;
  shortcutForAction?: (action: "approve" | "reject") => ReactNode;
}): React.JSX.Element {
  // `useSticky` is the existing "is the transcript pinned to the bottom" signal,
  // toggled the instant the user wheels/touches away — so it's exactly the
  // auto-collapse trigger (no new scroll listener on the fragile scroll model).
  const expanded = useSticky(sessionId);
  const isQuestion = item.requestKind === "question";
  const accent = isQuestion ? "primary.main" : "warning.main";
  const label = isQuestion ? "Answer needed" : "Permission required";
  const preferredActions = useMemo(() => {
    const find = (prefix: "allow" | "reject"): number => {
      const once = item.options.findIndex((option) =>
        option.kind.toLowerCase() === `${prefix}_once`
      );
      return once >= 0
        ? once
        : item.options.findIndex((option) => option.kind.toLowerCase().startsWith(prefix));
    };
    const approve = find("allow");
    const reject = find("reject");
    return { approve, reject };
  }, [item.options]);

  const respond = (optionId: string): void => {
    haptic();
    send({
      type: "permission",
      session_id: sessionId,
      request_id: item.requestId,
      option_id: optionId,
    });
  };

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
        fontFamily: "var(--cowboy-reading-font, inherit)",
        "& .MuiTypography-root, & .MuiButton-root": {
          fontFamily: "inherit",
        },
      }}
    >
      {shortcutForAction && preferredActions.approve >= 0 && (
        <button
          hidden
          type="button"
          tabIndex={-1}
          aria-hidden="true"
          data-desktop-permission-action="approve"
          onClick={(): void => respond(item.options[preferredActions.approve]!.optionId)}
        />
      )}
      {shortcutForAction && preferredActions.reject >= 0 && (
        <button
          hidden
          type="button"
          tabIndex={-1}
          aria-hidden="true"
          data-desktop-permission-action="reject"
          onClick={(): void => respond(item.options[preferredActions.reject]!.optionId)}
        />
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
          </Stack>
          <Box
            component="pre"
            sx={{
              m: 0,
              mb: 1.5,
              px: 1,
              py: 0.75,
              borderRadius: 1,
              bgcolor: "action.hover",
              fontFamily: "inherit",
              fontSize: 13,
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
            }}
          >
            {item.title}
          </Box>
          <Stack spacing={1}>
            {item.options.map((opt, index) => {
              const reject = opt.kind.startsWith("reject");
              const action = index === preferredActions.approve
                ? "approve"
                : index === preferredActions.reject
                ? "reject"
                : null;
              return (
                <Button
                  key={opt.optionId}
                  data-desktop-permission-action={action ?? undefined}
                  fullWidth
                  disableElevation
                  variant={reject ? "outlined" : "contained"}
                  color={reject ? "error" : "primary"}
                  endIcon={action ? shortcutForAction?.(action) : undefined}
                  onClick={(): void => respond(opt.optionId)}
                  sx={{
                    minHeight: { xs: 48, sm: 40 },
                    py: 0.75,
                    textTransform: "none",
                    fontSize: { xs: 15, sm: 14 },
                    lineHeight: 1.35,
                  }}
                >
                  {opt.name}
                </Button>
              );
            })}
          </Stack>
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
            ? <HelpOutlineRounded sx={{ fontSize: 18, color: accent, flexShrink: 0 }} />
            : (
              <WarningAmberRounded
                sx={{ fontSize: 18, color: accent, flexShrink: 0 }}
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
