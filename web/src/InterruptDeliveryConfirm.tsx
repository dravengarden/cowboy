import { Button, Typography } from "@mui/material";
import { ConfirmSheet } from "./Sheet";
import { NetworkButton } from "./NetworkActionFeedback";
import { isImeKeyEvent } from "./imeKey";

/** The second gesture is explicit; bare/held Enter never interrupts a turn. */
export function InterruptDeliveryConfirm(props: {
  open: boolean;
  preview: string;
  onClose: () => void;
  onConfirm: () => Promise<void>;
}): React.JSX.Element {
  return (
    <div
      onKeyDownCapture={(event): void => {
        if (!props.open || event.key !== "Enter") return;
        event.preventDefault();
        event.stopPropagation();
        if (
          !event.repeat && !isImeKeyEvent(event.nativeEvent) &&
          (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey
        ) {
          void props.onConfirm();
        }
      }}
    >
      <ConfirmSheet
        open={props.open}
        onClose={props.onClose}
        title="Interrupt & send?"
        actions={
          <>
            <Button color="inherit" onClick={props.onClose}>Cancel</Button>
            <NetworkButton
              color="warning"
              variant="contained"
              networkAction={props.onConfirm}
            >
              Interrupt & send
            </NetworkButton>
          </>
        }
      >
        <Typography variant="body2">
          Cancel the captured turn and submit this message once. Other queued
          messages and Pause remain unchanged.
        </Typography>
        <Typography
          variant="body2"
          sx={{
            mt: 1,
            whiteSpace: "pre-wrap",
            maxHeight: 160,
            overflow: "auto",
          }}
        >
          {props.preview}
        </Typography>
        <Typography variant="caption" color="text.secondary">
          ⌘/Ctrl+Enter confirms · Esc cancels
        </Typography>
      </ConfirmSheet>
    </div>
  );
}
