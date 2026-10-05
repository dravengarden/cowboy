import {
  Alert,
  Box,
  Button,
  Chip,
  IconButton,
  Stack,
  TextField,
  Tooltip,
  Typography,
} from "@mui/material";
import {
  AttachFile,
  HistoryOutlined,
  OpenInNew,
  SaveAlt,
} from "@mui/icons-material";
import {
  DownloadIcon,
  EllipsisIcon,
  HistoryIcon,
  KeyboardHideIcon,
  PanelLeftIcon,
} from "./draftChromeIcons";
import { alpha, type Theme } from "@mui/material/styles";
import { useKeyboardOpen } from "../keyboardInset";
import {
  lazy,
  type MutableRefObject,
  Suspense,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  type ComposerEditorHandle,
  type ComposerEditorSelection,
  PlatformComposerEditor,
} from "../composer/PlatformComposerEditor";
import { COMPOSER_COMMANDS_BY_ID } from "../composerCommands";
import { useComposerToolbar } from "../composerToolbarConfig";
import {
  type Attachment,
  fileToAttachment,
  imageTokensInText,
  pendingImageAttachment,
} from "../attachments";
import { seedInlineAttachments } from "../inlineImages";
import { useVimSetting } from "../vimSetting";
import { useSurfaceProfile } from "../surface/SurfaceProfile";
import { isImeKeyEvent } from "../imeKey";
import { getVimMode } from "../vimModeStore";
import { vimSinkAwaitsInput } from "../desktop/vim/vimSinkInput";
import { LeaderKeycap } from "../desktop/commands/DesktopKeycap";
import { DESKTOP_WORKSPACE_KEYS } from "../desktop/commands/workspaceShortcuts";
import { Sheet } from "../Sheet";
import { useBootReady } from "../useBootReady";
import { draftRepository, useDraftDocument } from "./store";
import { documentNotice } from "./DocumentNotifications";
import { type DraftDocument } from "./model";

const DesktopDraftToolbar = lazy(() =>
  import("../desktop/DesktopDraftToolbar")
);

// Group adjacent commands only: the user's configured order stays intact.
function toolbarGroup(id: string): string {
  if (["undo", "redo"].includes(id)) return "history";
  if (["sourceMode", "extensions"].includes(id)) return "editor";
  if (
    [
      "heading",
      "bulletList",
      "numberedList",
      "checklist",
      "quote",
      "codeBlock",
      "indent",
      "outdent",
    ].includes(id) || id.startsWith("heading")
  ) return "blocks";
  return "inline";
}

const desktopDraftActionSx = {
  flexShrink: 0,
  width: "2.25rem",
  height: "2.25rem",
  padding: "0.375rem",
  "& .MuiSvgIcon-root": { fontSize: "1.5rem" },
};

const positions = new Map<string, ComposerEditorSelection>();
/** Space under the formatting capsule. At rest it clears the home indicator;
 *  as the keyboard rises the App column pads by `--kb-inset`, so the same
 *  amount comes off here and the capsule moves continuously with the keyboard
 *  instead of first dropping onto the home indicator (no focus-driven jump). */
const DRAFT_MOBILE_BAR_CLEARANCE =
  "max(4px, calc(max(env(safe-area-inset-bottom, 0px), 8px) - var(--kb-inset, 0px)))";
export type DraftFlush = () => Promise<void>;

/** Mobile focus-on-writing chrome (Obsidian): the page owns its Sessions and
 *  actions controls at the top, and formatting rests at the bottom and rides
 *  above the keyboard. Create and Settings live in the Sessions drawer. */
export interface DraftMobileChrome {
  onOpenSessions: () => void;
  onMenu: () => void;
}

export function DraftEditor(
  { id, beforeLeave, onCopyToSession, mobileChrome }: {
    id: string;
    beforeLeave: MutableRefObject<DraftFlush>;
    onCopyToSession: () => void;
    mobileChrome?: DraftMobileChrome | undefined;
  },
): React.JSX.Element {
  const snapshot = useDraftDocument(id);
  const [epoch, setEpoch] = useState(0);
  useBootReady(snapshot.phase !== "loading");
  if (!snapshot.document) {
    return (
      <Box sx={{ p: 3 }}>
        <Typography>
          {snapshot.phase === "loading"
            ? "Opening draft…"
            : snapshot.error ?? "Draft unavailable"}
        </Typography>
        {snapshot.phase !== "loading" && (
          <Button onClick={() => void draftRepository().document(id).refresh()}>
            Retry
          </Button>
        )}
      </Box>
    );
  }
  return (
    <DraftEditingSession
      key={`${id}:${epoch}`}
      initial={snapshot.document}
      current={snapshot.document}
      phase={snapshot.phase}
      syncError={snapshot.error}
      beforeLeave={beforeLeave}
      onCopyToSession={onCopyToSession}
      mobileChrome={mobileChrome}
      onReload={() => setEpoch((n) => n + 1)}
    />
  );
}

function exportDraft(
  title: string,
  text: string,
  attachments: readonly Attachment[],
): void {
  let markdown = text;
  for (const attachment of attachments) {
    if (attachment.isImage && attachment.previewUrl?.startsWith("data:")) {
      markdown = markdown.replaceAll(
        `cowboy-att:${attachment.id}`,
        attachment.previewUrl,
      );
    }
  }
  const link = document.createElement("a");
  const url = URL.createObjectURL(
    new Blob([markdown], { type: "text/markdown;charset=utf-8" }),
  );
  link.href = url;
  link.download = `${title.replace(/[\\/:*?"<>|]/g, "-") || "Untitled"}.md`;
  link.click();
  globalThis.setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function DraftEditingSession(
  {
    initial,
    current,
    phase,
    syncError,
    beforeLeave,
    onCopyToSession,
    mobileChrome,
    onReload,
  }: {
    initial: DraftDocument;
    current: DraftDocument;
    phase: string;
    syncError: string | null;
    beforeLeave: MutableRefObject<DraftFlush>;
    onCopyToSession: () => void;
    mobileChrome?: DraftMobileChrome | undefined;
    onReload: () => void;
  },
): React.JSX.Element {
  const editor = useRef<ComposerEditorHandle | null>(null);
  const titleInput = useRef<HTMLInputElement | null>(null);
  const mountSeed = useRef(initial.body);
  const owner = draftRepository().document(initial.id);
  const [text, setText] = useState(initial.body);
  const textRef = useRef(initial.body);
  const [attachments, setAttachments] = useState<readonly Attachment[]>(() => {
    seedInlineAttachments(initial.attachments);
    return initial.attachments;
  });
  const attachmentsRef = useRef(attachments);
  const [title, setTitle] = useState(initial.title);
  const titleRef = useRef(initial.title);
  const savedTitle = useRef(initial.title);
  const bodyRevision = useRef(initial.body_revision);
  const titleRevision = useRef(initial.metadata_revision);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [history, setHistory] = useState<readonly DraftDocument[] | null>(null);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyPreview, setHistoryPreview] = useState<DraftDocument | null>(
    null,
  );
  const [readableWidth, setReadableWidth] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const dirtyRef = useRef(false);
  const saving = useRef<Promise<void>>(Promise.resolve());
  const encoding = useRef(new Set<Promise<void>>());
  const filePicker = useRef<HTMLInputElement>(null);
  const mounted = useRef(true);
  const composing = useRef(false);
  const desktop = useSurfaceProfile().kind === "desktop";
  const focusLayout = !desktop && mobileChrome !== undefined;
  const keyboardOpen = useKeyboardOpen();
  const vim = useVimSetting();
  const toolbar = useComposerToolbar();
  // The title is the document's first line (Obsidian inline title): `␣R`
  // selects it for renaming; `↑`/`k` on the body's first line enters it at
  // the end; `Enter`/`↓`/`Tab` returns to the body start and `Esc` returns
  // to where the body caret was (FOCUS.md "Draft document").
  const focusTitle = (select: boolean): void => {
    const input = titleInput.current;
    if (!input) return;
    input.focus({ preventScroll: true });
    if (select) input.select();
    else input.setSelectionRange(input.value.length, input.value.length);
  };
  const bodyCaretOnFirstLine = (): boolean => {
    const handle = editor.current;
    if (!handle) return false;
    const { head } = handle.getSelection();
    return !handle.getValue().slice(0, head).includes("\n");
  };
  const flush = async (): Promise<void> => {
    clearTimeout(timer.current);
    if (!dirtyRef.current && encoding.current.size === 0) return saving.current;
    await Promise.all(encoding.current);
    const body = textRef.current;
    const attached = attachmentsRef.current.filter((a) =>
      !a.pending &&
      (!a.isImage || imageTokensInText(body).some((token) => token.id === a.id))
    );
    const nextTitle = titleRef.current.trim() || "Untitled";
    const task = saving.current.catch(() => undefined).then(async () => {
      const saved = owner.get().document;
      if (!saved) throw new Error("Draft is unavailable");
      if (
        saved.body !== body ||
        JSON.stringify(saved.attachments) !== JSON.stringify(attached)
      ) {
        await owner.change(
          { type: "write", body, attachments: attached },
          bodyRevision.current,
        );
        bodyRevision.current++;
      }
      if (savedTitle.current !== nextTitle) {
        await owner.change(
          { type: "rename", title: nextTitle },
          titleRevision.current,
        );
        titleRevision.current++;
        savedTitle.current = nextTitle;
      }
      if (
        body === textRef.current &&
        nextTitle === (titleRef.current.trim() || "Untitled")
      ) {
        dirtyRef.current = false;
        if (mounted.current) {
          setDirty(false);
          setError(null);
        }
      }
    });
    saving.current = task;
    await task;
  };
  const flushRef = useRef(flush);
  flushRef.current = flush;
  beforeLeave.current = flush;
  const schedule = (): void => {
    dirtyRef.current = true;
    setDirty(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      void flushRef.current().catch((e: Error) => {
        if (mounted.current) setError(e.message);
      });
    }, 400);
  };
  useEffect(() => {
    mounted.current = true;
    const unload = (event: BeforeUnloadEvent): void => {
      if (dirtyRef.current || encoding.current.size) {
        event.preventDefault();
        event.returnValue = "";
      }
    };
    const pagehide = (): void => {
      void flushRef.current().catch(() => undefined);
    };
    globalThis.addEventListener("beforeunload", unload);
    globalThis.addEventListener("pagehide", pagehide);
    return () => {
      mounted.current = false;
      clearTimeout(timer.current);
      if (editor.current) {
        positions.set(initial.id, editor.current.getSelection());
      }
      void flushRef.current().catch((e: Error) => documentNotice(e.message));
      globalThis.removeEventListener("beforeunload", unload);
      globalThis.removeEventListener("pagehide", pagehide);
    };
  }, [initial.id]);
  useEffect(() => {
    if (phase !== "saved") return;
    if (
      titleRef.current === savedTitle.current &&
      current.metadata_revision > titleRevision.current
    ) {
      titleRevision.current = current.metadata_revision;
      savedTitle.current = current.title;
      titleRef.current = current.title;
      setTitle(current.title);
    }
  }, [current.metadata_revision, current.title, phase]);
  const attach = (files: File[]): void => {
    const selection = editor.current?.getSelection();
    const pending = files.filter((file) => file.type.startsWith("image/")).map((
      file,
    ) => ({ file, attachment: pendingImageAttachment(file) }));
    const placeholders = pending.map((item) => item.attachment);
    attachmentsRef.current = [...attachmentsRef.current, ...placeholders];
    setAttachments(attachmentsRef.current);
    if (placeholders.length) {
      editor.current?.insertImages(placeholders, selection);
    }
    const task = (async () => {
      for (const file of files) {
        const placeholder = pending.find((item) => item.file === file)
          ?.attachment;
        try {
          const attached = await fileToAttachment(file, placeholder?.id);
          attachmentsRef.current = [
            ...attachmentsRef.current.filter((a) => a.id !== attached.id),
            attached,
          ];
          seedInlineAttachments([attached]);
          if (mounted.current) {
            setAttachments(attachmentsRef.current);
            editor.current?.refreshImages();
          }
        } catch (cause) {
          if (placeholder) {
            attachmentsRef.current = attachmentsRef.current.filter((a) =>
              a.id !== placeholder.id
            );
            editor.current?.deleteImage(placeholder.id);
          }
          if (mounted.current) {
            setError(
              cause instanceof Error ? cause.message : "Could not attach file",
            );
          }
        }
      }
    })();
    encoding.current.add(task);
    void task.finally(() => {
      encoding.current.delete(task);
      if (mounted.current) schedule();
    });
  };
  const openHistory = async (): Promise<void> => {
    setHistoryLoading(true);
    try {
      await flush();
      const response = await fetch(
        `/api/drafts/${encodeURIComponent(initial.id)}/history`,
        { cache: "no-store" },
      );
      if (!response.ok) throw new Error("Could not load recovery history");
      setHistory(await response.json() as DraftDocument[]);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Recovery unavailable");
    } finally {
      setHistoryLoading(false);
    }
  };
  const recoverCopy = async (): Promise<void> => {
    const copy = await draftRepository().create(
      `${titleRef.current} (recovered)`,
      null,
      "document",
      textRef.current,
      attachmentsRef.current.filter((a) => !a.pending),
    );
    await owner.useRemote();
    dirtyRef.current = false;
    documentNotice("Your local version was preserved as a separate draft.");
    globalThis.location.hash = `drafts/${copy}`;
  };
  const changedElsewhere = !dirty && phase === "saved" &&
    (current.body !== textRef.current ||
      JSON.stringify(current.attachments) !==
        JSON.stringify(attachmentsRef.current));
  const saveStatus = dirty
    ? "Saving…"
    : phase === "saved"
    ? "Synced"
    : phase === "conflict"
    ? "Conflict · local copy kept"
    : phase === "error"
    ? "Needs attention"
    : "Saved on this device";
  const titleField = (
    <TextField
      variant="standard"
      value={title}
      placeholder="Untitled"
      fullWidth
      sx={{ flex: 1, minWidth: 0 }}
      inputRef={titleInput}
      inputProps={{ "aria-label": "Draft title", maxLength: 160 }}
      onKeyDown={(e) => {
        if (!desktop || isImeKeyEvent(e.nativeEvent)) return;
        if (e.metaKey || e.ctrlKey || e.altKey) return;
        const toStart = e.key === "Enter" || e.key === "ArrowDown" ||
          (e.key === "Tab" && !e.shiftKey);
        if (toStart) {
          e.preventDefault();
          editor.current?.focusSelection({ anchor: 0, head: 0 });
        } else if (e.key === "Escape") {
          // Leave the field for the body; never close a surrounding layer.
          e.preventDefault();
          e.stopPropagation();
          editor.current?.focus();
        }
      }}
      onChange={(e) => {
        setTitle(e.target.value);
        titleRef.current = e.target.value;
        schedule();
      }}
      onBlur={() => void flush().catch((e: Error) => setError(e.message))}
      slotProps={{
        input: {
          disableUnderline: true,
          sx: focusLayout
            ? {
              fontSize: "calc(1.75rem * var(--cowboy-font-scale, 1))",
              fontWeight: 700,
              lineHeight: 1.25,
              letterSpacing: "-0.01em",
            }
            : { fontSize: "1.4rem", fontWeight: 600 },
        },
      }}
    />
  );
  // Obsidian's floating controls: opaque capsules lifted by a soft, wide
  // shadow rather than outlined, with full-strength line icons, so the
  // writing surface itself carries no bars. Dark mode keeps a hairline
  // because a shadow cannot separate paper from a dark canvas.
  const floatingMaterialSx = {
    bgcolor: (t: Theme) =>
      t.palette.mode === "dark" ? t.palette.background.paper : "#fff",
    borderRadius: 999,
    border: (t: Theme) =>
      t.palette.mode === "dark" ? `1px solid ${t.palette.divider}` : "none",
    boxShadow: (t: Theme) =>
      `0 1px 2px ${
        alpha(t.palette.common.black, t.palette.mode === "dark" ? 0.5 : 0.06)
      }, 0 4px 18px ${
        alpha(t.palette.common.black, t.palette.mode === "dark" ? 0.45 : 0.07)
      }`,
    "& .MuiIconButton-root": { color: "text.primary" },
    "& .MuiSvgIcon-root": { fontSize: "1.375rem" },
  } as const;
  const formatCommands = toolbar
    .filter((id) => !["mention", "slash", "attach"].includes(id))
    .flatMap((id) => {
      const command = COMPOSER_COMMANDS_BY_ID[id];
      return command ? [{ id, command }] : [];
    });
  const keepEditorFocus = {
    onPointerDown: (e: { preventDefault: () => void }) => e.preventDefault(),
    onMouseDown: (e: { preventDefault: () => void }) => e.preventDefault(),
  };
  // Always present: a scrolling capsule of formatting that rests at the bottom
  // and rides above the keyboard. Hide keyboard appears only while a keyboard
  // is actually up, not merely while the body has focus: focus arrives before
  // the keyboard, and iOS can dismiss the keyboard without blurring.
  const focusToolbar = (
    <Stack
      data-draft-mobile-toolbar
      direction="row"
      alignItems="center"
      spacing={1}
      sx={{
        px: 1,
        py: 0.75,
        flexShrink: 0,
        mb: DRAFT_MOBILE_BAR_CLEARANCE,
      }}
    >
      <Stack
        data-draft-format-toolbar
        direction="row"
        alignItems="center"
        sx={{
          ...floatingMaterialSx,
          flex: 1,
          minWidth: 0,
          overflowX: "auto",
          px: 0.5,
          scrollbarWidth: "none",
          "&::-webkit-scrollbar": { display: "none" },
        }}
      >
        {formatCommands.map(({ id, command }) => (
          <IconButton
            key={id}
            data-draft-tool
            aria-label={command.label}
            {...keepEditorFocus}
            onClick={() => {
              if (!composing.current && editor.current) {
                command.run({
                  editor: editor.current,
                  attach: () => filePicker.current?.click(),
                });
              }
            }}
            sx={{ flexShrink: 0, width: 44, height: 44 }}
          >
            {command.icon}
          </IconButton>
        ))}
        <IconButton
          aria-label="Attach file"
          {...keepEditorFocus}
          onClick={() => filePicker.current?.click()}
          sx={{ flexShrink: 0, width: 44, height: 44 }}
        >
          <AttachFile />
        </IconButton>
      </Stack>
      {keyboardOpen && (
      <IconButton
        data-draft-hide-keyboard
        aria-label="Hide keyboard"
        {...keepEditorFocus}
        onClick={() => {
          const active = globalThis.document.activeElement;
          if (active instanceof HTMLElement) active.blur();
        }}
        sx={{
          ...floatingMaterialSx,
          color: "text.primary",
          width: 48,
          height: 48,
          flexShrink: 0,
        }}
      >
        <KeyboardHideIcon />
      </IconButton>
      )}
    </Stack>
  );
  const toolbarView = (
    <Stack
      direction="row"
      alignItems="center"
      sx={{
        borderTop: 1,
        borderColor: "divider",
        px: desktop ? "0.75rem" : 0.5,
        py: desktop ? "0.5rem" : 0.5,
        gap: desktop ? "0.5rem" : 0,
        flexWrap: desktop ? "wrap" : "nowrap",
        overflowX: desktop ? "visible" : "auto",
        flexShrink: 0,
        pb: desktop ? "0.5rem" : "max(4px, env(safe-area-inset-bottom))",
      }}
    >
      <Box
        data-draft-format-toolbar
        sx={{
          display: desktop ? "flex" : "contents",
          alignItems: "center",
          flexWrap: desktop ? "wrap" : "nowrap",
          gap: desktop ? "0.125rem" : 0,
          minWidth: 0,
          maxWidth: "100%",
        }}
      >
        {(() => {
          const visible = toolbar.filter((id) =>
            !["mention", "slash", "attach"].includes(id)
          );
          const groups: string[][] = [];
          for (const id of visible) {
            const previous = groups.at(-1);
            if (
              !desktop ||
              (previous && toolbarGroup(previous[0]!) === toolbarGroup(id))
            ) {
              if (previous) previous.push(id);
              else groups.push([id]);
            } else groups.push([id]);
          }
          return groups.map((ids, index) => (
            <Box
              key={ids.join(":")}
              data-draft-tool-group
              sx={{
                display: "contents",
              }}
            >
              {ids
                .map((id, commandIndex) => {
                  const command = COMPOSER_COMMANDS_BY_ID[id];
                  return command
                    ? (
                      <Tooltip key={id} title={command.label}>
                        <IconButton
                          data-draft-tool
                          aria-label={command.label}
                          onPointerDown={(e) => e.preventDefault()}
                          onMouseDown={(e) => e.preventDefault()}
                          onClick={() => {
                            if (!composing.current && editor.current) {
                              command.run({
                                editor: editor.current,
                                attach: () => filePicker.current?.click(),
                              });
                            }
                          }}
                          sx={{
                            flexShrink: 0,
                            ...(desktop
                              ? desktopDraftActionSx
                              : { width: 44, height: 44 }),
                            ...(desktop && index > 0 && commandIndex === 0
                              ? {
                                ml: "0.5rem",
                                "&::before": {
                                  content: '""',
                                  position: "absolute",
                                  left: "-0.25rem",
                                  top: "0.5rem",
                                  bottom: "0.5rem",
                                  width: "1px",
                                  bgcolor: "divider",
                                  pointerEvents: "none",
                                },
                              }
                              : {}),
                          }}
                        >
                          {command.icon}
                        </IconButton>
                      </Tooltip>
                    )
                    : null;
                })}
            </Box>
          ));
        })()}
        <Tooltip title="Attach file">
          <IconButton
            aria-label="Attach file"
            sx={desktop ? desktopDraftActionSx : undefined}
            onClick={() => filePicker.current?.click()}
          >
            <AttachFile />
          </IconButton>
        </Tooltip>
      </Box>

      <Box
        sx={{
          display: desktop ? "flex" : "contents",
          alignItems: "center",
          gap: "0.75rem",
          ml: "auto",
          flexShrink: 0,
          ...(desktop
            ? {
              flexWrap: "wrap",
              justifyContent: "flex-end",
              maxWidth: "100%",
            }
            : {}),
        }}
      >
        {desktop && (
          <Button
            size="small"
            color="inherit"
            sx={{
              textTransform: "none",
              fontSize: "0.8125rem",
              minHeight: "2.25rem",
              px: "0.75rem",
              bgcolor: readableWidth ? "action.selected" : undefined,
            }}
            aria-pressed={readableWidth}
            onClick={() => setReadableWidth((v) => !v)}
          >
            Readable width
          </Button>
        )}
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{
            ml: desktop ? 0 : "auto",
            px: desktop ? "0.25rem" : 1,
            whiteSpace: "nowrap",
          }}
          role="status"
        >
          {dirty
            ? "Saving…"
            : phase === "saved"
            ? "Synced"
            : phase === "conflict"
            ? "Conflict · local copy kept"
            : phase === "error"
            ? "Needs attention"
            : "Saved on this device"}
        </Typography>
      </Box>
    </Stack>
  );
  return (
    <Stack
      sx={{
        flex: 1,
        minHeight: 0,
        minWidth: 0,
        ...(desktop
          ? { containerType: "inline-size", containerName: "draft-editor" }
          : {}),
        ...(focusLayout && { position: "relative" }),
      }}
      data-draft-editor={initial.id}
      data-desktop-region={desktop ? "prompt.composer" : undefined}
      onCompositionStartCapture={() => {
        composing.current = true;
      }}
      onCompositionEndCapture={() => {
        composing.current = false;
      }}
    >
      {focusLayout && mobileChrome
        ? (
          <Box sx={{ px: "18px", pt: 1, flexShrink: 0 }}>
            <Stack
              direction="row"
              alignItems="center"
              justifyContent="space-between"
              sx={{ mx: "-8px" }}
            >
              <IconButton
                aria-label="Open sessions"
                onClick={mobileChrome.onOpenSessions}
                sx={{
                  ...floatingMaterialSx,
                  color: "text.primary",
                  width: 48,
                  height: 48,
                }}
              >
                <PanelLeftIcon />
              </IconButton>
              <Stack
                direction="row"
                alignItems="center"
                sx={{ ...floatingMaterialSx, height: 48, px: 0.5 }}
              >
                <IconButton
                  disabled={historyLoading}
                  aria-label="Recovery history"
                  onClick={() => void openHistory()}
                >
                  <HistoryIcon />
                </IconButton>
                <IconButton
                  aria-label="Export Markdown"
                  onClick={() =>
                    exportDraft(title, textRef.current, attachmentsRef.current)}
                >
                  <DownloadIcon />
                </IconButton>
                <IconButton
                  aria-label="Draft actions"
                  onClick={() =>
                    void flush().then(mobileChrome.onMenu).catch((e: Error) =>
                      setError(e.message)
                    )}
                >
                  <EllipsisIcon />
                </IconButton>
              </Stack>
            </Stack>
            <Box sx={{ mt: 2 }}>{titleField}</Box>
            <Typography variant="caption" color="text.disabled" role="status">
              {saveStatus}
            </Typography>
          </Box>
        )
        : (
          <Stack
            direction="row"
            alignItems="center"
            spacing={desktop ? "0.25rem" : 1}
            sx={{
              ...(desktop
                ? {
                  "& .MuiIconButton-root": desktopDraftActionSx,
                  "@container draft-editor (max-width: 20rem)": {
                    "& .MuiIconButton-root": { display: "none" },
                  },
                }
                : {}),
              px: desktop ? "1rem" : 1,
              py: desktop ? "0.375rem" : 0.5,
              borderBottom: 1,
              borderColor: "divider",
            }}
          >
            {titleField}
            {desktop && (
              <Box
                component="span"
                data-draft-title-shortcut
                title="Rename: Space D R (Cmd/Alt+K D R from a text field), or ↑ from the first line"
                sx={{ display: "inline-flex", flexShrink: 0 }}
              >
                <LeaderKeycap
                  leaderKey={DESKTOP_WORKSPACE_KEYS.rename}
                />
              </Box>
            )}
            {/* Desktop shows these actions once, in the bottom document bar
                with their leader slots; Mobile keeps them beside the title. */}
            {!desktop && (
              <>
              <Tooltip title="Copy to Session drafts">
                <IconButton
                  aria-label="Copy to Session drafts"
                  onClick={() =>
                    void flush().then(onCopyToSession).catch((e: Error) =>
                      setError(e.message)
                    )}
                >
                  <OpenInNew />
                </IconButton>
              </Tooltip>
              <Tooltip title="Recovery history">
                <IconButton
                  disabled={historyLoading}
                  aria-label="Recovery history"
                  onClick={() => void openHistory()}
                >
                  <HistoryOutlined />
                </IconButton>
              </Tooltip>
              <Tooltip title="Export Markdown">
                <IconButton
                  aria-label="Export Markdown"
                  onClick={() =>
                    exportDraft(title, textRef.current, attachmentsRef.current)}
                >
                  <SaveAlt />
                </IconButton>
              </Tooltip>
              </>
            )}
          </Stack>
        )}
      {current.deleted && (
        <Alert
          severity="warning"
          action={
            <Button
              color="inherit"
              onClick={() =>
                void owner.change({ type: "restore" }).catch((e: Error) =>
                  setError(e.message)
                )}
            >
              Restore draft
            </Button>
          }
        >
          This draft is in Trash. Your open text has been kept; restore it
          before continuing.
        </Alert>
      )}
      {(error || syncError) && (
        <Alert
          severity={phase === "conflict" ? "warning" : "error"}
          action={phase === "conflict"
            ? (
              <Button
                color="inherit"
                onClick={() =>
                  void recoverCopy().catch((e: Error) => setError(e.message))}
              >
                Keep mine as copy
              </Button>
            )
            : (
              <Button
                color="inherit"
                onClick={() =>
                  void owner.retry().then(flush).catch((e: Error) =>
                    setError(e.message)
                  )}
              >
                Retry
              </Button>
            )}
        >
          {error ?? syncError}
        </Alert>
      )}
      {changedElsewhere && (
        <Alert
          severity="info"
          action={
            <Button
              color="inherit"
              onClick={() => {
                dirtyRef.current = false;
                onReload();
              }}
            >
              Load latest
            </Button>
          }
        >
          This draft was updated on another device.
        </Alert>
      )}
      <Box
        sx={{
          display: "flex",
          flex: 1,
          minHeight: 0,
          minWidth: 0,
          px: desktop ? 1.5 : 0.5,
          // An empty page hints quietly (Obsidian): both engines use the
          // disabled tone, including the touch textarea before CM6 mounts.
          "& textarea::placeholder, & .cm-placeholder": {
            color: "text.disabled",
            opacity: 1,
          },
        }}
        data-draft-body
        data-mobile-drawer-idle-swipe={focusLayout ? "true" : undefined}
        data-desktop-region={desktop ? "prompt.composer" : undefined}
        onKeyDownCapture={(e) => {
          // ↑ in Insert, or a plain Vim Normal `k`, on the first line moves
          // into the title. A pending Vim command (`dk`, `ck`) keeps its key.
          if (!desktop || isImeKeyEvent(e.nativeEvent)) return;
          if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
          const sink = e.target instanceof Element &&
            e.target.matches("[data-vim-command-sink]");
          const up = e.key === "ArrowUp" ||
            (sink && e.code === "KeyK" && getVimMode() === "normal" &&
              !vimSinkAwaitsInput(e.target));
          if (!up || !bodyCaretOnFirstLine()) return;
          e.preventDefault();
          e.stopPropagation();
          focusTitle(false);
        }}
      >
        <Box
          sx={{
            display: "flex",
            flex: 1,
            minHeight: 0,
            minWidth: 0,
            width: "100%",
            "& [data-mobile-native-editor]": {
              flex: 1,
              height: "100%",
              maxHeight: "none",
            },
            "& textarea[data-mobile-native-textarea]": {
              height: "100% !important",
              maxHeight: "none !important",
            },
            maxWidth: readableWidth ? "85ch" : "none",
            mx: "auto",
          }}
        >
          <PlatformComposerEditor
            ref={editor}
            documentId={initial.id}
            value={mountSeed.current}
            nativeValue={text}
            onChange={(value) => {
              textRef.current = value;
              setText(value);
              schedule();
            }}
            {...(positions.has(initial.id)
              ? { initialSelection: positions.get(initial.id)!.head }
              : {})}
            onPasteFiles={attach}
            placeholder="Start writing…"
            borderless
            fill
            flushRightScrollbar={desktop}
            vim={vim}
          />
        </Box>
      </Box>
      {attachments.some((a) => !a.isImage) && (
        <Stack direction="row" sx={{ px: 1, gap: 0.5, flexWrap: "wrap" }}>
          {attachments.filter((a) =>
            !a.isImage
          ).map((a) => (
            <Chip
              key={a.id}
              label={a.name}
              onDelete={() => {
                attachmentsRef.current = attachmentsRef.current.filter((item) =>
                  item.id !== a.id
                );
                setAttachments(attachmentsRef.current);
                schedule();
              }}
            />
          ))}
        </Stack>
      )}
      {desktop
        ? (
          <Suspense fallback={toolbarView}>
            <DesktopDraftToolbar
              fallback={toolbarView}
              toolbar={toolbar}
              status={saveStatus}
              writable={!current.deleted && phase !== "conflict"}
              historyLoading={historyLoading}
              readableWidth={readableWidth}
              onReadableWidth={() => setReadableWidth((v) => !v)}
              onFormat={(id) => {
                if (!composing.current && editor.current) {
                  COMPOSER_COMMANDS_BY_ID[id]?.run({
                    editor: editor.current,
                    attach: () => filePicker.current?.click(),
                  });
                }
              }}
              onSave={() =>
                void flush().catch((e: Error) => setError(e.message))}
              onAttach={() => filePicker.current?.click()}
              onCopy={() =>
                void flush().then(onCopyToSession).catch((e: Error) =>
                  setError(e.message)
                )}
              onHistory={() => void openHistory()}
              onExport={() =>
                exportDraft(title, textRef.current, attachmentsRef.current)}
              onRename={() => focusTitle(true)}
            />
          </Suspense>
        )
        : focusLayout
        ? (
          focusToolbar
        )
        : toolbarView}
      <input
        ref={filePicker}
        type="file"
        multiple
        hidden
        onChange={(e) => {
          attach([...e.target.files ?? []]);
          e.target.value = "";
        }}
      />
      {history && (
        <Sheet
          open
          title="Recovery history"
          onClose={() => {
            setHistory(null);
            setHistoryPreview(null);
          }}
          actions={
            <Button
              onClick={() => {
                setHistory(null);
                setHistoryPreview(null);
              }}
            >
              Close
            </Button>
          }
        >
          <Typography variant="body2" color="text.secondary">
            Previous saved versions. Restoring creates a new version.
          </Typography>
          <Box sx={{ maxHeight: "50dvh", overflowY: "auto" }}>
            {history.length === 0 && (
              <Typography sx={{ py: 2 }}>No earlier versions yet.</Typography>
            )}
            {history.map((version) => (
              <Button
                key={version.revision}
                fullWidth
                onClick={() => setHistoryPreview(version)}
              >
                {new Date(version.updated_at_ms).toLocaleString()} ·{" "}
                {version.body.length.toLocaleString()} characters
              </Button>
            ))}
            {historyPreview && (
              <>
                <Box
                  component="pre"
                  sx={{
                    whiteSpace: "pre-wrap",
                    overflowWrap: "anywhere",
                    fontSize: "0.9rem",
                  }}
                >
                  {historyPreview.body}
                </Box>
                <Button
                  onClick={() =>
                    void owner.change({
                      type: "write",
                      body: historyPreview.body,
                      attachments: historyPreview.attachments,
                    }).then(() => {
                      dirtyRef.current = false;
                      setHistory(null);
                      onReload();
                    }).catch((e: Error) => setError(e.message))}
                >
                  Restore this version
                </Button>
              </>
            )}
          </Box>
        </Sheet>
      )}
    </Stack>
  );
}
