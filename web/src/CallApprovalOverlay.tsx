import { desktopSize } from "./surface/desktopSize";
import { type ReactNode, useState } from "react";
import { alpha, Box, Button, Stack, Typography } from "@mui/material";
import AccountTree from "@mui/icons-material/AccountTree";
import {
  type CallApproval,
  type CallApprovalDecision,
  callApprovalReason,
  callApprovalTitle,
  decideCallApproval,
} from "./callApproval";
import { agentLabel } from "./agentTools";
import { requestStickToBottom, useSticky } from "./stickyStore";
import { frostedPanel, frostedPill } from "./frostedGlass";
import { haptic } from "./haptic";

// Agent calls waiting for a person, in the same slot and material as the
// tool-permission overlay: the agent is blocked on this answer. "Allow once"
// admits only the waiting requests and keeps the session's policy (and its
// token cost) as it is; "Allow for this session" turns calls on in the
// session's Tools. Desktop A / R reuse the permission shortcuts.
export function CallApprovalOverlay({
  approval,
  sessionId,
  shortcutForAction,
}: {
  approval: CallApproval;
  sessionId: string;
  shortcutForAction?: (action: "approve" | "reject") => ReactNode;
}): React.JSX.Element {
  const expanded = useSticky(sessionId);
  const [busy, setBusy] = useState<CallApprovalDecision | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const title = callApprovalTitle(approval);
  const answer = (decision: CallApprovalDecision): void => {
    if (busy) return;
    haptic();
    setBusy(decision);
    setFailure(null);
    decideCallApproval(sessionId, decision).catch((reason: unknown) => {
      setFailure(
        reason instanceof Error ? reason.message : "Could not answer",
      );
    }).finally(() => setBusy(null));
  };
  const shown = approval.items.slice(0, 3);
  const more = approval.requests - shown.length;
  return (
    <Box
      data-call-approval-overlay
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
        fontSize: "1rem",
        "& .MuiTypography-root, & .MuiButton-root": {
          fontFamily: "inherit",
        },
      }}
    >
      {shortcutForAction && (
        <>
          <button
            hidden
            type="button"
            tabIndex={-1}
            aria-hidden="true"
            data-desktop-permission-action="approve"
            onClick={(): void => answer("once")}
          />
          <button
            hidden
            type="button"
            tabIndex={-1}
            aria-hidden="true"
            data-desktop-permission-action="reject"
            onClick={(): void => answer("decline")}
          />
        </>
      )}
      {expanded
        ? (
          <Box
            role="dialog"
            aria-label={title}
            sx={(t) => ({
              pointerEvents: "auto",
              width: "100%",
              maxWidth: 460,
              p: 1.5,
              borderRadius: 2.5,
              ...frostedPanel(t),
              backgroundImage: `linear-gradient(0deg, ${
                alpha(
                  t.palette.primary.main,
                  t.palette.mode === "dark" ? 0.16 : 0.12,
                )
              }, ${
                alpha(
                  t.palette.primary.main,
                  t.palette.mode === "dark" ? 0.16 : 0.12,
                )
              })`,
            })}
          >
            <Stack
              direction="row"
              spacing={1}
              alignItems="center"
              sx={{ color: "primary.main", mb: 0.5 }}
            >
              <AccountTree fontSize="small" sx={{ flexShrink: 0 }} />
              <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                {title}
              </Typography>
            </Stack>
            <Typography
              variant="body2"
              sx={{ color: "text.secondary", mb: 1 }}
            >
              {callApprovalReason(approval)}{" "}
              Calls start read-only reviewers and use tokens.
            </Typography>
            <Stack
              component="ul"
              spacing={0.5}
              sx={{
                m: 0,
                mb: 1.5,
                px: 1,
                py: 0.75,
                listStyle: "none",
                borderRadius: 1,
                bgcolor: "action.hover",
                color: "text.primary",
                fontSize: "0.875rem",
              }}
            >
              {shown.map((item, index) => (
                <Box
                  component="li"
                  key={index}
                  sx={{ display: "flex", gap: 1, minWidth: 0 }}
                >
                  <Box
                    component="span"
                    sx={{ fontWeight: 650, flexShrink: 0 }}
                  >
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
            <Stack spacing={1}>
              <Button
                fullWidth
                disableElevation
                variant="contained"
                disabled={busy !== null}
                data-desktop-permission-action={shortcutForAction
                  ? "approve"
                  : undefined}
                endIcon={shortcutForAction?.("approve")}
                onClick={(): void => answer("once")}
                sx={buttonSx}
              >
                {approval.requests === 1 ? "Allow once" : "Allow these calls"}
              </Button>
              <Button
                fullWidth
                disableElevation
                variant="outlined"
                disabled={busy !== null}
                onClick={(): void => answer("session")}
                sx={buttonSx}
              >
                Allow for this session
              </Button>
              <Button
                fullWidth
                disableElevation
                variant="outlined"
                color="error"
                disabled={busy !== null}
                data-desktop-permission-action={shortcutForAction
                  ? "reject"
                  : undefined}
                endIcon={shortcutForAction?.("reject")}
                onClick={(): void => answer("decline")}
                sx={buttonSx}
              >
                Decline
              </Button>
            </Stack>
            {failure && (
              <Typography
                role="alert"
                variant="body2"
                sx={{ color: "error.main", mt: 1 }}
              >
                {failure}
              </Typography>
            )}
          </Box>
        )
        : (
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
              ...frostedPill(t, t.palette.primary.main),
            })}
          >
            <AccountTree
              sx={{
                fontSize: desktopSize(18),
                color: "primary.main",
                flexShrink: 0,
              }}
            />
            <Typography
              variant="body2"
              sx={{
                fontWeight: 600,
                color: "primary.main",
                whiteSpace: "nowrap",
              }}
            >
              Call approval needed
            </Typography>
          </Stack>
        )}
    </Box>
  );
}

const buttonSx = {
  minHeight: { xs: 48, sm: 40 },
  py: 0.75,
  textTransform: "none",
  fontSize: { xs: "1rem", sm: "0.9375rem" },
  lineHeight: 1.35,
} as const;
