import { lazy, type ReactNode, Suspense } from "react";
import { Box, Button, CircularProgress, Stack, Typography } from "@mui/material";
import { ChatBubbleOutline } from "@mui/icons-material";
import { openMobileProduct } from "../appPagerMotion";

// Keep Review's repository, diff and file tooling out of the Mobile shell's
// initial module graph. The pager mounts both pages, so this import warms in
// the background while Agent is already usable.
const ReviewApp = lazy(async () => {
  const module = await import("./ReviewApp");
  return { default: module.ReviewApp };
});

function ReviewLoading(): React.JSX.Element {
  return (
    <Stack
      data-mobile-review-loading
      sx={{
        height: "100%",
        minHeight: 0,
        bgcolor: "background.default",
        color: "text.primary",
        pt: "var(--cowboy-system-top-clearance, 0px)",
      }}
    >
      <Box
        sx={{
          px: 2,
          minHeight: 52,
          display: "flex",
          alignItems: "center",
          borderBottom: 1,
          borderColor: "divider",
        }}
      >
        <Typography variant="body2">Review</Typography>
      </Box>
      <Box role="status" sx={{ flex: 1, minHeight: 0, p: 2 }}>
        <Stack direction="row" spacing={1} alignItems="center">
          <CircularProgress size={20} aria-label="Opening review" />
          <Typography variant="body2" color="text.secondary">
            Opening review…
          </Typography>
        </Stack>
      </Box>
      <Box
        sx={{
          px: 1.5,
          pt: 0.5,
          pb: "max(env(safe-area-inset-bottom), 12px)",
          borderTop: 1,
          borderColor: "divider",
        }}
      >
        <Button
          color="inherit"
          startIcon={<ChatBubbleOutline />}
          onClick={() => openMobileProduct("agent")}
          sx={{ minHeight: 44 }}
        >
          Agent
        </Button>
      </Box>
    </Stack>
  );
}

/** Loading a Review chunk must never suspend Agent or the product pager. */
export function MobileReviewBoundary(
  { children }: { children: ReactNode },
): React.JSX.Element {
  return <Suspense fallback={<ReviewLoading />}>{children}</Suspense>;
}

export function MobileReviewPage(props: {
  active: boolean;
  onDrawerOpenChange: (open: boolean) => void;
}): React.JSX.Element {
  return (
    <MobileReviewBoundary>
      <ReviewApp {...props} />
    </MobileReviewBoundary>
  );
}
