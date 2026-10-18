import { IconButton } from "@mui/material";
import { EditOutlined } from "@mui/icons-material";
import type { RefObject } from "react";
import type { SxProps, Theme } from "@mui/material/styles";
import type { ComposerEditorHandle } from "../ComposerEditor";
import { useKeyboardOpen } from "../keyboardInset";
import { KeyboardHideIcon } from "./draftChromeIcons";

/** A stable slot: keyboard visibility must not resize the formatting strip or
 * re-render the document while WebKit is starting an input transaction. */
export function DraftKeyboardToggle({ editor, sx, disabled }: {
  editor: RefObject<ComposerEditorHandle | null>;
  sx: SxProps<Theme>;
  disabled: boolean;
}): React.JSX.Element {
  const open = useKeyboardOpen();
  return (
    <IconButton
      data-draft-keyboard-toggle
      data-draft-hide-keyboard={open ? "true" : undefined}
      aria-label={open ? "Hide keyboard" : "Edit draft"}
      disabled={!open && disabled}
      onPointerDown={(event) => event.preventDefault()}
      onMouseDown={(event) => event.preventDefault()}
      onClick={(event) => {
        if (open) {
          const active = document.activeElement;
          // A delayed click must never dismiss a newer overlay's input.
          if (
            active instanceof HTMLElement &&
            active.closest("[data-draft-editor]") ===
              event.currentTarget.closest("[data-draft-editor]")
          ) active.blur();
        } else {
          // Preserve the saved caret; focusEnd would jump to the document end.
          editor.current?.focus();
        }
      }}
      sx={sx}
    >
      {open ? <KeyboardHideIcon /> : <EditOutlined />}
    </IconButton>
  );
}
