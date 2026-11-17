// The product confirmation for a view's call of a tool that is not
// read-only (docs/widget-apps-mcp.md §4): the tool and its exact arguments,
// bound by the Controller to this run. An uncertain outcome is reported to the
// view and never retried.

import { Box, Button, Typography } from "@mui/material";
import { ConfirmSheet } from "../../Sheet";

export type PendingCall = {
  call: { tool: string; arguments: unknown };
  answer: (confirmed: boolean) => void;
};

export default function McpCallConfirm({
  pending,
  binding,
  onClose,
}: {
  pending: PendingCall;
  binding: string;
  onClose: () => void;
}): React.JSX.Element {
  const answer = (confirmed: boolean): void => {
    pending.answer(confirmed);
    onClose();
  };
  return (
    <ConfirmSheet
      open
      onClose={(): void => answer(false)}
      title="Allow this call?"
      actions={
        <>
          <Button onClick={(): void => answer(false)} sx={{ textTransform: "none" }}>
            Decline
          </Button>
          <Button
            variant="contained"
            onClick={(): void => answer(true)}
            sx={{ textTransform: "none" }}
          >
            Call {pending.call.tool}
          </Button>
        </>
      }
    >
      <Box sx={{ display: "grid", gap: 1 }}>
        <Typography variant="body2">
          The view from <b>{binding}</b> asks to call <b>{pending.call.tool}</b>, which may change
          things. If its outcome is unknown, Cowboy will not try again.
        </Typography>
        <Box
          component="pre"
          data-mcp-call-arguments=""
          sx={{
            m: 0,
            p: 1,
            borderRadius: 1,
            bgcolor: "action.hover",
            fontSize: 12,
            maxHeight: 240,
            overflow: "auto",
            whiteSpace: "pre-wrap",
            overflowWrap: "anywhere",
          }}
        >
          {JSON.stringify(pending.call.arguments, null, 2)}
        </Box>
      </Box>
    </ConfirmSheet>
  );
}
