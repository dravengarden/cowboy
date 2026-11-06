import { Alert, Box, Button } from "@mui/material";
import { useRef, useState } from "react";
import { refreshReviewChecks, useReviewChecks } from "./reviewChecksStore";
import { sendCheckRequest } from "./checkRequest";

/** User-triggered checks only. An uncertain reply retains its operation ID. */
export function ReviewCheckActions({ sessionId }: { sessionId: string }): React.JSX.Element | null {
  const checks = useReviewChecks(sessionId);
  const operations = useRef(new Map<string, { operation: string; epoch: string }>());
  const active = useRef(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  if (!checks?.allowed || !checks.runnable || !checks.epoch || checks.declared.length === 0) {
    return null;
  }
  const start = async (check: string) => {
    if (active.current) return;
    active.current = true;
    setPending(true);
    setError(undefined);
    const key = `${sessionId}:${check}`;
    const request = operations.current.get(key) ?? {
      operation: crypto.randomUUID(),
      epoch: checks.epoch!,
    };
    operations.current.set(key, request);
    try {
      const result = await sendCheckRequest(sessionId, check, request);
      if (result.forget) operations.current.delete(key);
      setError(result.message);
      refreshReviewChecks(sessionId);
    } catch (cause) {
      setError(
        `${
          cause instanceof Error ? cause.message : "Check request failed."
        } Retry to query the same request.`,
      );
    } finally {
      active.current = false;
      setPending(false);
    }
  };
  return (
    <Box sx={{ px: 1, flexShrink: 0, maxHeight: "min(192px, 35dvh)", overflowY: "auto" }}>
      <Box sx={{ display: "flex", flexWrap: "wrap", gap: 1 }}>
        {checks.declared.map((check) => (
          <Button
            key={check.id}
            data-review-check-action={check.id}
            sx={{ minHeight: 44, minWidth: 44 }}
            disabled={
              pending ||
              checks.runs.some((run) => run.check === check.id && run.state === "running")
            }
            onClick={() => void start(check.id)}
          >
            Run {check.title ?? check.id}
          </Button>
        ))}
      </Box>
      {error && <Alert severity="warning">{error}</Alert>}
    </Box>
  );
}
