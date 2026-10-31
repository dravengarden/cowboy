import { Alert, Button, Stack, Typography } from "@mui/material";
import Undo from "@mui/icons-material/Undo";
import { glyphSize } from "./surface/desktopSize";
import { configRevertSummary, useConfigRevert } from "./configRevert";
import type { ConfigOption } from "./protocol";

/** "Changed from Opus · High · Back" in Session Settings until the next
 *  message is sent. Renders nothing when there is nothing to go back to. */
export function ConfigRevertBack(
  { sessionId, options }: {
    sessionId: string;
    options: readonly ConfigOption[] | undefined;
  },
): React.JSX.Element | null {
  const { revert, restore, restoring, error } = useConfigRevert(sessionId);
  const summary = configRevertSummary(revert.values, options);
  if (!summary && !error) return null;
  return (
    <Stack spacing={0.5} data-config-revert sx={{ minWidth: 0 }}>
      {summary && (
        <Stack direction="row" alignItems="center" spacing={0.75} sx={{ minWidth: 0 }}>
          <Undo sx={{ fontSize: glyphSize(16), color: "text.secondary", flexShrink: 0 }} />
          <Typography
            variant="caption"
            color="text.secondary"
            noWrap
            sx={{ minWidth: 0 }}
            title={`Changed from ${summary}. Back is available until your next message.`}
          >
            Changed from {summary}
          </Typography>
          <Button
            size="small"
            data-config-revert-back
            disabled={restoring}
            onClick={() => void restore()}
            sx={{ flexShrink: 0, minWidth: 0, px: 0.75, textTransform: "none" }}
          >
            {restoring ? "Going back…" : "Back"}
          </Button>
        </Stack>
      )}
      {error && <Alert severity="warning">{error}</Alert>}
    </Stack>
  );
}
