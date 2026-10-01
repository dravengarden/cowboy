import {
  Box,
  Button,
  CircularProgress,
  Stack,
  Typography,
} from "@mui/material";
import { useEffect, useState } from "react";

/** Local observation time, not a claim about remote Provider progress. */
export function ProviderAuthenticationProgress(
  { failed, onCheck, label = "Getting your sign-in link…" }: {
    failed: boolean;
    onCheck: () => void;
    label?: string;
  },
) {
  const [startedAt] = useState(() => Date.now());
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => {
      setElapsed(Math.max(0, Math.floor((Date.now() - startedAt) / 1000)));
    }, 1000);
    return () => clearInterval(timer);
  }, [startedAt]);
  return (
    <Box sx={{ border: 1, borderColor: "divider", borderRadius: 2, p: 2 }}>
      <Stack direction="row" spacing={1.5} alignItems="center" role="status">
        {!failed && (
          <CircularProgress
            size={22}
            disableShrink
            aria-label="Preparing sign-in"
          />
        )}
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography variant="body2" fontWeight={600}>
            {failed ? "Sign-in status unavailable" : label}
          </Typography>
          <Typography variant="body2" color="text.secondary">
            {failed
              ? "See the message above for the next step."
              : elapsed >= 30
              ? "This is taking longer than expected. You can check again or cancel sign-in."
              : "The next step will appear here when it is ready."}
          </Typography>
        </Box>
      </Stack>
      <Stack
        direction="row"
        alignItems="center"
        justifyContent="space-between"
        sx={{ mt: 1 }}
      >
        <Typography variant="caption" color="text.secondary" aria-live="off">
          {elapsed}s elapsed
        </Typography>
        <Button size="small" onClick={onCheck}>Check status</Button>
      </Stack>
    </Box>
  );
}
