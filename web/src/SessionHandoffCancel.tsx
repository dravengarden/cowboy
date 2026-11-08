import { Button, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { blocksToAttachments } from "./attachments";
import { announceDraft, appendDraftText, getDraft, setDraft } from "./draftStore";
import type { ContentBlock } from "./protocol";
import { cancelHandoff } from "./sessionContext";
import { requestPickSession } from "./sessionPickRequest";

/** The queued first message of a continuation carries its send operation. */
const BOOTSTRAP_PREFIX = "context-";

export function handoffOperation(cmid: string | undefined): string | undefined {
  return cmid?.startsWith(BOOTSTRAP_PREFIX) ? cmid.slice(BOOTSTRAP_PREFIX.length) : undefined;
}

/** Put the taken-back message where it was written, after anything typed
 *  there since. Attachments go through the draft store, which a mounted
 *  Composer adopts; the text is appended through the same path, so a Composer
 *  that mounted before this reply still shows it. */
function returnToComposer(id: string, text: string, content: ContentBlock[]): void {
  const attachments = blocksToAttachments(content, text);
  if (attachments.length > 0) {
    const draft = getDraft(id);
    setDraft(id, { ...draft, attachments: [...draft.attachments, ...attachments] });
    announceDraft(id);
  }
  appendDraftText(id, text);
}

/** While a continuation's first message still waits for its agent, undo the
 *  whole switch: the original comes back with this message in its Composer
 *  and the new session goes to Trash. Once the agent has it, Stop applies. */
export function SessionHandoffCancel(
  { sessionId, operation }: { sessionId: string; operation: string },
): React.JSX.Element {
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState("");
  const cancel = (): void => {
    setBusy(true);
    setFailure("");
    void cancelHandoff(sessionId, operation)
      .then((cancelled) => {
        returnToComposer(cancelled.source_session_id, cancelled.text, cancelled.content);
        requestPickSession(cancelled.source_session_id, cancelled.session ?? undefined);
      })
      .catch((error: unknown) =>
        setFailure(error instanceof Error ? error.message : "Could not cancel")
      )
      .finally(() => setBusy(false));
  };
  return (
    <Stack direction="row" spacing={1} alignItems="center" sx={{ minWidth: 0 }}>
      {failure && (
        <Typography variant="caption" color="error" sx={{ minWidth: 0 }}>
          {failure}
        </Typography>
      )}
      <Button
        data-handoff-cancel
        size="small"
        disabled={busy}
        title="Go back to the original session with this message"
        onClick={cancel}
        sx={{ flexShrink: 0, minWidth: 0, minHeight: 44, px: 1.25, textTransform: "none" }}
      >
        {busy ? "Cancelling…" : "Cancel"}
      </Button>
    </Stack>
  );
}
