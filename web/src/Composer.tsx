import { useState } from "react";
import { Box, IconButton, Stack, TextField, Typography } from "@mui/material";
import { Send, Stop } from "@mui/icons-material";
import { send } from "./store";
import type { Status } from "./protocol";

// Cmd/Ctrl + Enter = send. Plain Enter = newline.
//
// Why this way (not the reverse): pasting multi-line code / prompts is a
// daily action; making plain Enter send would shred any pasted snippet that
// contains a newline. ChatGPT/Claude.ai/Cursor all default to "Enter =
// newline + Cmd-Enter sends" for the same reason. Touch keyboards inherit
// the same model — their Enter key inserts a newline and the user taps the
// send button.
export function Composer({
  sessionId,
  status,
}: {
  sessionId: string;
  status: Status;
}): React.JSX.Element {
  const [text, setText] = useState("");
  const busy = status === "busy";
  const dead = status === "exited" || status === "crashed";
  const sendable = !!text.trim() && !dead;

  function submit(): void {
    if (!sendable) return;
    send({ type: "prompt", session_id: sessionId, text: text.trimEnd() });
    setText("");
  }

  return (
    <Box sx={{ p: { xs: 1, sm: 1.5 }, borderTop: 1, borderColor: "divider" }}>
      <Stack direction="row" spacing={1} alignItems="flex-end">
        <TextField
          fullWidth
          multiline
          minRows={1}
          maxRows={12}
          size="small"
          placeholder={dead ? "Session ended" : "Message the agent…"}
          value={text}
          disabled={dead}
          onChange={(e): void => setText(e.target.value)}
          onKeyDown={(e): void => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              submit();
            }
          }}
          slotProps={{
            input: {
              sx: {
                fontFamily:
                  "system-ui, -apple-system, Segoe UI, Roboto, sans-serif",
                fontSize: { xs: 14, sm: 14 },
              },
            },
          }}
        />
        {busy ? (
          <IconButton
            color="error"
            aria-label="cancel"
            onClick={(): void => send({ type: "cancel", session_id: sessionId })}
          >
            <Stop />
          </IconButton>
        ) : (
          <IconButton
            color="primary"
            aria-label="send"
            disabled={!sendable}
            onClick={submit}
          >
            <Send />
          </IconButton>
        )}
      </Stack>
      <Typography
        variant="caption"
        color="text.disabled"
        sx={{ display: "block", mt: 0.5, textAlign: "right", fontSize: 11 }}
      >
        Enter = newline · ⌘/Ctrl + Enter = send
      </Typography>
    </Box>
  );
}
