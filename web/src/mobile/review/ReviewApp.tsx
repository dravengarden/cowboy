import { ArrowBack, RateReviewOutlined } from "@mui/icons-material";
import { Box, Chip, IconButton, Stack, Typography } from "@mui/material";
import { useActiveWorkspaceBinding } from "../../controlPlane";

export function ReviewApp({
  onOpenAgent,
}: {
  onOpenAgent: () => void;
}): React.JSX.Element {
  const workspace = useActiveWorkspaceBinding();
  return (
    <Stack
      sx={{
        height: "100%",
        minWidth: 0,
        bgcolor: "background.default",
        color: "text.primary",
        pt: "env(safe-area-inset-top, 0px)",
        pb: "env(safe-area-inset-bottom, 0px)",
      }}
    >
      <Stack
        component="header"
        direction="row"
        alignItems="center"
        spacing={0.5}
        sx={{
          minHeight: 56,
          px: 1,
          borderBottom: 1,
          borderColor: "divider",
          bgcolor: "background.default",
        }}
      >
        <IconButton aria-label="Back to Agent" onClick={onOpenAgent}>
          <ArrowBack />
        </IconButton>
        <Typography component="h1" variant="h6" sx={{ fontWeight: 650 }}>
          Code Review
        </Typography>
      </Stack>

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
    </Stack>
  );
}
