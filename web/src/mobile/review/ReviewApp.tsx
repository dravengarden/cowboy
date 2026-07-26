import { RateReviewOutlined } from "@mui/icons-material";
import { Box, Chip, Stack, Toolbar, Typography } from "@mui/material";
import { useActiveWorkspaceBinding } from "../../controlPlane";
import { ReviewSettings } from "./ReviewSettings";

export function ReviewApp(): React.JSX.Element {
  const workspace = useActiveWorkspaceBinding();
  return (
    <Stack
      sx={{
        height: "100%",
        minWidth: 0,
        bgcolor: "background.default",
        color: "text.primary",
        pt: "env(safe-area-inset-top, 0px)",
      }}
    >
      <Stack
        component="main"
        alignItems="center"
        justifyContent="center"
        spacing={2}
        sx={{
          flex: 1,
          minHeight: 0,
          px: 4,
          textAlign: "center",
        }}
      >
        <Box
          sx={{
            display: "grid",
            placeItems: "center",
            width: 64,
            height: 64,
            borderRadius: 3,
            color: "primary.main",
            bgcolor: "action.selected",
          }}
        >
          <RateReviewOutlined sx={{ fontSize: 32 }} />
        </Box>
        <Stack spacing={0.75}>
          <Typography variant="h6" sx={{ fontWeight: 650 }}>
            Review the active worktree
          </Typography>
          <Typography color="text.secondary">
            The read-only code surface will use the same workspace as the active
            Agent session.
          </Typography>
        </Stack>
        {workspace && (
          <Stack spacing={0.75} alignItems="center" sx={{ maxWidth: "100%" }}>
            <Chip
              size="small"
              label={workspace.title || workspace.sessionId}
              color="primary"
              variant="outlined"
            />
            <Typography
              variant="caption"
              color="text.secondary"
              sx={{
                maxWidth: "100%",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {workspace.cwd}
            </Typography>
          </Stack>
        )}
        <Typography variant="caption" color="text.secondary">
          Swipe right to return to Agent
        </Typography>
      </Stack>
      <Box
        component="nav"
        aria-label="Code Review controls"
        sx={{
          pb: "max(calc(env(safe-area-inset-bottom) - 18px), 12px)",
          pl: "env(safe-area-inset-left, 0px)",
          pr: "env(safe-area-inset-right, 0px)",
          borderTop: 1,
          borderColor: "divider",
          bgcolor: "background.default",
        }}
      >
        <Toolbar
          variant="dense"
          sx={{
            minHeight: 44,
            "@media (min-width: 600px)": { minHeight: 44 },
          }}
        >
          <ReviewSettings />
        </Toolbar>
      </Box>
    </Stack>
  );
}
