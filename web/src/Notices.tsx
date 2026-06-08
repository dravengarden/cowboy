import type { ReactNode } from "react";
import { Alert, Box, Button, Collapse, Stack } from "@mui/material";

// A reusable, MUI-`Alert`-driven notice surface (design §J). Lives at the top of
// the content area and renders a stack of dismissible/actionable notices. The
// first producer is the no-inference-key warning (App computes it from
// `inferenceConfig`); future producers (rate-limit, degraded-judge, …) just push
// more `Notice`s. Kept presentational — all state lives in the caller.
export interface Notice {
  id: string;
  severity: "warning" | "info" | "error" | "success";
  message: ReactNode;
  /** Optional inline action (e.g. "Configure" → open the Info sheet). */
  actionLabel?: string;
  onAction?: () => void;
}

export function Notices({ notices }: { notices: Notice[] }): React.JSX.Element | null {
  if (notices.length === 0) return null;
  return (
    <Box sx={{ px: 1.5, pt: 1 }}>
      <Stack spacing={1}>
        {notices.map((n) => (
          // `in appear` animates the entrance; React unmounts on removal (the
          // producing condition flipping), which is fine for these low-churn
          // notices — no exit choreography needed.
          <Collapse key={n.id} in appear>
            <Alert
              severity={n.severity}
              variant="outlined"
              action={
                n.onAction && n.actionLabel ? (
                  <Button color="inherit" size="small" onClick={n.onAction} sx={{ textTransform: "none" }}>
                    {n.actionLabel}
                  </Button>
                ) : undefined
              }
              sx={{ alignItems: "center", borderRadius: 2 }}
            >
              {n.message}
            </Alert>
          </Collapse>
        ))}
      </Stack>
    </Box>
  );
}
