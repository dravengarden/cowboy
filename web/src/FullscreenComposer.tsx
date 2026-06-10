import { type ReactNode, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  AppBar,
  Box,
  IconButton,
  Stack,
  Toolbar,
  Tooltip,
  useTheme,
} from "@mui/material";
import {
  AlternateEmail,
  AttachFile,
  CloseFullscreen,
  Code,
  FormatBold,
  FormatItalic,
  FormatListBulleted,
  FormatQuote,
  InsertLink,
  Send,
  Tag,
  Title,
} from "@mui/icons-material";
import { ComposerEditor, type ComposerEditorHandle } from "./ComposerEditor";
import type { AvailableCommand } from "./protocol";
import { haptic } from "./haptic";

// Brand-new full-screen mobile compose surface (NOT a DetentSheet): a fixed
// 100dvh overlay modeled on Obsidian's mobile note editor — a light top bar
// (collapse + send), a full-height CM6 + mdlive live-preview canvas, and a
// SELECTION-AWARE markdown toolbar pinned just above the keyboard. The editor is
// the same engine as every other surface, so markdown stays the literal value and
// nothing re-serializes on open/close. The native iOS keyboard accessory bar is
// suppressed by cowboy's Tauri keyboard-bar swizzle; this toolbar replaces it.
export function FullscreenComposer({
  value,
  onChange,
  onSubmit,
  onSaveDraft,
  onCollapse,
  onAttach,
  onPasteFiles,
  sessionId,
  commands,
  placeholder,
  sendable,
  attachmentsSlot,
}: {
  value: string;
  onChange: (v: string) => void;
  onSubmit: () => void;
  onSaveDraft: () => void;
  onCollapse: () => void;
  onAttach: () => void;
  onPasteFiles: (files: File[]) => void;
  sessionId: string;
  commands: () => AvailableCommand[];
  placeholder: string;
  sendable: boolean;
  attachmentsSlot?: ReactNode;
}): React.JSX.Element {
  const theme = useTheme();
  const editorRef = useRef<ComposerEditorHandle>(null);
  // The toolbar swaps INSERT actions (no selection) ↔ WRAP actions (selection).
  const [hasSelection, setHasSelection] = useState(false);

  // Raise the keyboard on open. iOS can drop a single early focus while the
  // overlay is still painting, so focus a few times across the first frames
  // (cheap, idempotent) — the editor lands focused with the caret at the end.
  useEffect(() => {
    const timers = [0, 120, 320].map((d) =>
      globalThis.setTimeout(() => editorRef.current?.focusEnd(), d)
    );
    return (): void => timers.forEach((t) => globalThis.clearTimeout(t));
  }, []);

  const act = (fn: () => void): (() => void) => () => {
    haptic();
    fn();
  };

  // Each action keeps the keyboard up (the handle methods re-focus the editor).
  const insertActions = (
    <>
      <ToolBtn title="Heading" onClick={act(() => editorRef.current?.cycleHeading())}>
        <Title />
      </ToolBtn>
      <ToolBtn
        title="Bulleted list"
        onClick={act(() => editorRef.current?.toggleLinePrefix("- "))}
      >
        <FormatListBulleted />
      </ToolBtn>
      <ToolBtn
        title="Quote"
        onClick={act(() => editorRef.current?.toggleLinePrefix("> "))}
      >
        <FormatQuote />
      </ToolBtn>
      <ToolBtn title="Inline code" onClick={act(() => editorRef.current?.wrap("`", "`"))}>
        <Code />
      </ToolBtn>
      <ToolBtn title="Mention" onClick={act(() => editorRef.current?.insertTrigger("@"))}>
        <AlternateEmail />
      </ToolBtn>
      <ToolBtn
        title="Slash command"
        onClick={act(() => editorRef.current?.insertTrigger("/"))}
      >
        <Tag />
      </ToolBtn>
      <ToolBtn title="Attach" onClick={act(onAttach)}>
        <AttachFile />
      </ToolBtn>
    </>
  );

  const wrapActions = (
    <>
      <ToolBtn title="Bold" onClick={act(() => editorRef.current?.wrap("**", "**"))}>
        <FormatBold />
      </ToolBtn>
      <ToolBtn title="Italic" onClick={act(() => editorRef.current?.wrap("*", "*"))}>
        <FormatItalic />
      </ToolBtn>
      <ToolBtn title="Inline code" onClick={act(() => editorRef.current?.wrap("`", "`"))}>
        <Code />
      </ToolBtn>
      <ToolBtn title="Link" onClick={act(() => editorRef.current?.insertLink())}>
        <InsertLink />
      </ToolBtn>
    </>
  );

  // Portal to <body>: the composer is rendered deep inside the app's flex layout,
  // whose ancestors form stacking contexts (the bottom navbar paints over an
  // inline fixed child). A portal escapes them so the overlay truly covers the app.
  return createPortal(
    <Box
      sx={{
        position: "fixed",
        inset: 0,
        zIndex: theme.zIndex.modal,
        bgcolor: "background.default",
        display: "flex",
        flexDirection: "column",
        // Reserve the on-screen keyboard's height so the toolbar (last child) sits
        // ABOVE it (a fixed element's bottom otherwise hides under the keyboard on
        // iOS — the layout viewport doesn't shrink). When the keyboard is down,
        // fall back to the home-indicator safe inset.
        pb: "max(var(--kb-inset, 0px), env(safe-area-inset-bottom, 0px))",
      }}
    >
      <AppBar
        position="static"
        color="transparent"
        elevation={0}
        sx={{
          bgcolor: "background.default",
          borderBottom: 1,
          borderColor: "divider",
          pt: "env(safe-area-inset-top, 0px)",
        }}
      >
        <Toolbar variant="dense" sx={{ minHeight: 48, gap: 1 }}>
          <Tooltip title="Collapse">
            <IconButton aria-label="collapse editor" onClick={act(onCollapse)}>
              <CloseFullscreen />
            </IconButton>
          </Tooltip>
          <Box sx={{ flex: 1 }} />
          <Tooltip title="Send">
            <span>
              <IconButton
                aria-label="send"
                color="primary"
                disabled={!sendable}
                onClick={act(onSubmit)}
              >
                <Send />
              </IconButton>
            </span>
          </Tooltip>
        </Toolbar>
      </AppBar>

      {attachmentsSlot != null && (
        <Box sx={{ px: 1.5, pt: 1 }}>{attachmentsSlot}</Box>
      )}

      {/* The writing canvas — fills the space between the top bar and the toolbar. */}
      <Box sx={{ flex: 1, minHeight: 0, p: 1.5, display: "flex", flexDirection: "column" }}>
        <ComposerEditor
          ref={editorRef}
          value={value}
          onChange={onChange}
          onSubmit={onSubmit}
          onSaveDraft={onSaveDraft}
          sessionId={sessionId}
          commands={commands}
          placeholder={placeholder}
          onPasteFiles={onPasteFiles}
          onSelectionChange={setHasSelection}
          borderless
          fill
          vim={false}
          onEscape={(): boolean => {
            onCollapse();
            return true;
          }}
        />
      </Box>

      {/* Selection-aware markdown toolbar, pinned just above the keyboard. Insert
          actions with no selection (H/list/quote/code/@//attach), wrap actions
          when text is selected (bold/italic/code/link). */}
      <Stack
        direction="row"
        alignItems="center"
        spacing={0.5}
        sx={{
          px: 1,
          py: 0.5,
          borderTop: 1,
          borderColor: "divider",
          bgcolor: "background.paper",
          overflowX: "auto",
          // Don't let the row wrap; let it scroll on a narrow phone.
          flexWrap: "nowrap",
        }}
      >
        {hasSelection ? wrapActions : insertActions}
      </Stack>
    </Box>,
    document.body,
  );
}

// One toolbar button — fixed 40px tap target, secondary tint, glyph forced to a
// touch-friendly size (beats the global MuiIconButton override).
function ToolBtn({
  title,
  onClick,
  children,
}: {
  title: string;
  onClick: () => void;
  children: ReactNode;
}): React.JSX.Element {
  return (
    <Tooltip title={title}>
      <IconButton
        aria-label={title}
        onClick={onClick}
        sx={{
          width: 40,
          height: 40,
          flexShrink: 0,
          color: "text.secondary",
          "& .MuiSvgIcon-root": { fontSize: "1.375rem" },
        }}
      >
        {children}
      </IconButton>
    </Tooltip>
  );
}
