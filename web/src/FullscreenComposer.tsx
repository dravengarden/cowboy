import { type ReactNode, type RefObject, useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import {
  AppBar,
  Box,
  Divider,
  IconButton,
  Stack,
  Toolbar,
  Tooltip,
  useTheme,
} from "@mui/material";
import {
  AlternateEmail,
  AttachFile,
  CheckBoxOutlined,
  CloseFullscreen,
  Code,
  DataObject,
  FormatBold,
  FormatItalic,
  FormatListBulleted,
  FormatListNumbered,
  FormatQuote,
  InsertLink,
  Redo,
  Send,
  StrikethroughS,
  Tag,
  Title,
  Undo,
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
  editorRef,
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
  // The SHARED composer editor handle. Must be the same ref the parent's
  // `addFiles`/submit/clear use — else paste-an-image here inserts the inline
  // token into nobody (the parent's inline editor is unmounted while fullscreen is
  // open), so the image attaches + sends but shows NO inline thumbnail.
  editorRef: RefObject<ComposerEditorHandle | null>;
}): React.JSX.Element {
  const theme = useTheme();

  // UNCONTROLLED, exactly like the inline composer: freeze the open-time text as a
  // one-shot seed and let `onChange` flow text OUT only. Passing the live
  // `value={text}` (what we used to do) makes @uiw/react-codemirror reconcile the
  // doc on EVERY keystroke — and worse, on every IME composition update — which
  // bounces the iOS caret and corrupts mid-composition pinyin ("状态错乱"). The
  // inline editor's own comment warns against `value={text}` for exactly this; the
  // fullscreen surface must follow the same rule. This component remounts on each
  // open (parent gates it behind `composeFs`), so the frozen seed is always the
  // current in-progress text at open time; on close the parent syncs it back.
  const seed = useRef(value).current;

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

  // A FIXED, comprehensive Obsidian-style toolbar (no selection-swap). The
  // wrap/inline actions work with OR without a selection (the handle inserts an
  // empty marker pair at the caret when nothing is selected). Scrolls
  // horizontally on a narrow phone. Each action re-focuses the editor (keyboard
  // stays up). A thin divider groups history / format / block / insert.
  const e = (): ComposerEditorHandle | null => editorRef.current;
  const sep = (
    <Divider orientation="vertical" flexItem sx={{ my: 0.75, mx: 0.25 }} />
  );
  const actions = (
    <>
      <ToolBtn title="Undo" onClick={act(() => e()?.undo())}>
        <Undo />
      </ToolBtn>
      <ToolBtn title="Redo" onClick={act(() => e()?.redo())}>
        <Redo />
      </ToolBtn>
      {sep}
      <ToolBtn title="Heading" onClick={act(() => e()?.cycleHeading())}>
        <Title />
      </ToolBtn>
      <ToolBtn title="Bold" onClick={act(() => e()?.wrap("**", "**"))}>
        <FormatBold />
      </ToolBtn>
      <ToolBtn title="Italic" onClick={act(() => e()?.wrap("*", "*"))}>
        <FormatItalic />
      </ToolBtn>
      <ToolBtn title="Strikethrough" onClick={act(() => e()?.wrap("~~", "~~"))}>
        <StrikethroughS />
      </ToolBtn>
      <ToolBtn title="Inline code" onClick={act(() => e()?.wrap("`", "`"))}>
        <Code />
      </ToolBtn>
      <ToolBtn title="Link" onClick={act(() => e()?.insertLink())}>
        <InsertLink />
      </ToolBtn>
      {sep}
      <ToolBtn title="Bulleted list" onClick={act(() => e()?.toggleLinePrefix("- "))}>
        <FormatListBulleted />
      </ToolBtn>
      <ToolBtn title="Numbered list" onClick={act(() => e()?.toggleLinePrefix("1. "))}>
        <FormatListNumbered />
      </ToolBtn>
      <ToolBtn title="Checklist" onClick={act(() => e()?.toggleLinePrefix("- [ ] "))}>
        <CheckBoxOutlined />
      </ToolBtn>
      <ToolBtn title="Quote" onClick={act(() => e()?.toggleLinePrefix("> "))}>
        <FormatQuote />
      </ToolBtn>
      <ToolBtn title="Code block" onClick={act(() => e()?.insertCodeBlock())}>
        <DataObject />
      </ToolBtn>
      {sep}
      <ToolBtn title="Mention" onClick={act(() => e()?.insertTrigger("@"))}>
        <AlternateEmail />
      </ToolBtn>
      <ToolBtn title="Slash command" onClick={act(() => e()?.insertTrigger("/"))}>
        <Tag />
      </ToolBtn>
      <ToolBtn title="Attach" onClick={act(onAttach)}>
        <AttachFile />
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
          value={seed}
          onChange={onChange}
          onSubmit={onSubmit}
          onSaveDraft={onSaveDraft}
          sessionId={sessionId}
          commands={commands}
          placeholder={placeholder}
          onPasteFiles={onPasteFiles}
          borderless
          fill
          vim={false}
          onEscape={(): boolean => {
            onCollapse();
            return true;
          }}
        />
      </Box>

      {/* Fixed Obsidian-style markdown toolbar, pinned just above the keyboard;
          scrolls horizontally on a narrow phone. */}
      <Stack
        direction="row"
        alignItems="center"
        spacing={0.25}
        sx={{
          px: 1,
          py: 0.5,
          borderTop: 1,
          borderColor: "divider",
          bgcolor: "background.paper",
          overflowX: "auto",
          flexWrap: "nowrap",
        }}
      >
        {actions}
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
