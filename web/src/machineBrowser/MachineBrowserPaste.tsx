/** "Paste from this device", the neko way: Safari and Firefox cannot sync a
 * clipboard by themselves, so the panel offers a small sheet with a text
 * field. It is prefilled from the device clipboard only when the browser
 * allows reading it inside the tap that opened it; otherwise the owner
 * pastes (or types) into the field. Send sets the remote clipboard and
 * presses Ctrl+V on exactly the viewer that was bound when it opened. */
import { Button, Stack, TextField, Typography } from "@mui/material";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { ConfirmSheet } from "../Sheet";
import type { ViewerControls } from "./browserChrome";

interface PasteDraft {
  /** The viewer the text goes to; Send refuses once another one is bound. */
  controls: ViewerControls;
  text: string;
}

/** `start` must run inside the menu tap: the clipboard read needs it. */
export function useDevicePaste(controls: ViewerControls | undefined): {
  start: () => void;
  dialog: ReactNode;
} {
  const [draft, setDraft] = useState<PasteDraft | undefined>(undefined);
  const generation = useRef(0);
  const current = draft !== undefined && draft.controls === controls;
  useEffect(() => {
    if (draft && !current) setDraft(undefined);
  }, [draft, current]);
  const close = (): void => {
    generation.current += 1;
    setDraft(undefined);
  };
  const start = (): void => {
    if (!controls) return;
    generation.current += 1;
    const opened = generation.current;
    setDraft({ controls, text: "" });
    const clipboard = globalThis.navigator?.clipboard;
    if (typeof clipboard?.readText !== "function") return;
    clipboard.readText().then(
      (text) => {
        if (generation.current !== opened || !text) return;
        setDraft((previous) => (previous && !previous.text ? { ...previous, text } : previous));
      },
      () => {},
    );
  };
  const send = (): void => {
    if (!draft || !current || !draft.text) return;
    draft.controls.paste(draft.text);
    close();
  };
  const dialog = draft ? (
    <ConfirmSheet
      open
      onClose={close}
      title="Paste into the browser"
      actions={
        <Stack direction="row" spacing={1}>
          <Button onClick={close}>Cancel</Button>
          <Button variant="contained" disabled={!current || !draft.text} onClick={send}>
            Send
          </Button>
        </Stack>
      }
    >
      <Typography variant="body2" color="text.secondary">
        Paste or type the text here. Send puts it on the remote clipboard and presses Ctrl+V in the
        browser.
      </Typography>
      <TextField
        multiline
        minRows={3}
        maxRows={8}
        fullWidth
        value={draft.text}
        onChange={(event) =>
          setDraft((previous) => previous && { ...previous, text: event.target.value })
        }
        slotProps={{
          htmlInput: {
            "aria-label": "Text to paste",
            autoCapitalize: "off",
            autoCorrect: "off",
            spellCheck: false,
            style: { fontSize: 16 },
          },
        }}
      />
    </ConfirmSheet>
  ) : null;
  return { start, dialog };
}
