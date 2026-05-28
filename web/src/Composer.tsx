import { useState } from "react";
import { Box, IconButton, Stack, TextField } from "@mui/material";
import { Send, Stop } from "@mui/icons-material";
import { send } from "./store";
import type { Status } from "./protocol";

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

  function submit(): void {
    const t = text.trim();
    if (!t || dead) return;
    send({ type: "prompt", session_id: sessionId, text: t });
    setText("");
  }

  return (
    <Box sx={{ p: { xs: 1, sm: 1.5 }, borderTop: 1, borderColor: "divider" }}>
      <Stack direction="row" spacing={1} alignItems="flex-end">
        <TextField
          fullWidth
          multiline
          maxRows={8}
          size="small"
          placeholder={dead ? "Session ended" : "Message the agent…"}
          value={text}
          disabled={dead}
          onChange={(e): void => setText(e.target.value)}
          onKeyDown={(e): void => {
            // Enter sends; Shift+Enter newline. (Touch keyboards: use the
            // button — Enter inserts a newline there.)
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              submit();
            }
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
          <IconButton color="primary" aria-label="send" disabled={dead} onClick={submit}>
            <Send />
          </IconButton>
        )}
      </Stack>
    </Box>
  );
}
