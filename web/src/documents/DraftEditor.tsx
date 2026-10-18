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
  PanelLeftIcon,
} from "./draftChromeIcons";
import { alpha, type Theme } from "@mui/material/styles";
import { DraftKeyboardToggle } from "./DraftKeyboardToggle";
import { DraftFormatStrip } from "./DraftFormatStrip";
import { observeDraftViewport } from "./draftViewport";
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
import { imeOwnsEditable, isImeKeyEvent } from "../imeKey";
import { getVimMode } from "../vimModeStore";
import { vimSinkAwaitsInput } from "../desktop/vim/vimSinkInput";
import { LeaderKeycap } from "../desktop/commands/DesktopKeycap";
import {
  DESKTOP_DOCUMENT_KEYS,
  desktopWorkspaceSequence,
} from "../desktop/commands/workspaceShortcuts";
import { HintTooltip } from "../HintTooltip";
import {
  enterInputNormal,
  inputVimField,
  isInputVimNormal,
  useInputVimMode,
} from "../desktop/vim/inputVim";
import { Kbd } from "../Kbd";
import { Sheet } from "../Sheet";
import { useBootReady } from "../useBootReady";
import { draftRepository, useDraftDocument } from "./store";
import { documentNotice } from "./DocumentNotifications";
import {
  draftAttachmentsFit,
  type DraftContent,
  draftContent,
  type DraftDocument,
  type DraftSnapshotSummary,
  mergeDraftContent,
  sameDraftContent,
} from "./model";
import { DraftMergeError } from "./repository";

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
  touch: boolean,
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
  const name = `${title.replace(/[\\/:*?"<>|]/g, "-") || "Untitled"}.md`;
  const blob = new Blob([markdown], { type: "text/markdown;charset=utf-8" });
  // A touch WebView (the iOS shell, a home-screen PWA) silently ignores a
  // download link. Its share sheet saves to Files or hands off to an app;
  // it must open inside this same tap.
  const file = new File([blob], name, { type: "text/markdown" });
  if (touch && navigator.canShare?.({ files: [file] })) {
    void navigator.share({ files: [file], title }).catch((e: Error) => {
      if (e.name !== "AbortError") documentNotice(e.message);
    });
    return;
  }
  const link = document.createElement("a");
  const url = URL.createObjectURL(blob);
  link.href = url;
  link.download = name;
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
  const canvas = useRef<HTMLDivElement | null>(null);
  const documentScroll = useRef<HTMLDivElement | null>(null);
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
  // The document content this editor's text was last in sync with. Writes
  // name it as their merge ancestor, and newer content (another device, or a
  // server-side merge) is merged into the live text against it.
  const shadow = useRef<DraftContent>(draftContent(initial));
  const writing = useRef(false);
  const reconcileTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const titleRevision = useRef(initial.metadata_revision);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The History list carries no content; a version's text and attachments
  // arrive when it is opened, or earlier by a low-priority prefetch.
  const [history, setHistory] = useState<
    readonly DraftSnapshotSummary[] | null
  >(null);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyPreview, setHistoryPreview] = useState<
    {
      summary: DraftSnapshotSummary;
      document: DraftDocument | null;
      error: string | null;
    } | null
  >(null);
  const historyPrefetch = useRef<AbortController | null>(null);
  useEffect(() => () => historyPrefetch.current?.abort(), []);
  const closeHistory = (): void => {
    historyPrefetch.current?.abort();
    setHistory(null);
    setHistoryPreview(null);
  };
  const previewVersion = (summary: DraftSnapshotSummary): void => {
    setHistoryPreview({ summary, document: null, error: null });
    void draftRepository().historySnapshot(initial.id, summary.revision).then(
      (document) =>
        setHistoryPreview((open) =>
          open?.summary.revision === summary.revision
            ? { ...open, document }
            : open
        ),
      (cause: Error) =>
        setHistoryPreview((open) =>
          open?.summary.revision === summary.revision
            ? { ...open, error: cause.message }
            : open
        ),
    );
  };
  const [readableWidth, setReadableWidth] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const dirtyRef = useRef(false);
  const saving = useRef<Promise<void>>(Promise.resolve());
  const encoding = useRef(new Set<Promise<void>>());
  const filePicker = useRef<HTMLInputElement>(null);
  const mounted = useRef(true);
  const composing = useRef(false);
  const compositionEndedAt = useRef(0);
  const desktop = useSurfaceProfile().kind === "desktop";
  const focusLayout = !desktop && mobileChrome !== undefined;
  useEffect(() => {
    const viewport = focusLayout ? documentScroll.current : canvas.current;
    if (!viewport) return;
    return observeDraftViewport(viewport, () => {
      if (
        !imeOwnsEditable(
          composing.current,
          compositionEndedAt.current,
          Date.now(),
        ) && editor.current?.hasFocus()
      ) {
        editor.current.revealSelection();
      }
    });
  }, [focusLayout]);
  const vim = useVimSetting();
  const toolbar = useComposerToolbar();
  // The title is the document's first line (Obsidian inline title): `␣T`
  // or a Vim Normal `k` on the body's first line puts the cursor on its end
  // in Normal; `↑` in Insert enters it typing. `Enter`/`↓`/`Tab` (and `j`
  // in Normal) return to the body start, `Esc` to where the body caret was
  // (FOCUS.md "Draft document"). With Vim on the title is a Vim field.
  const titleMode = useInputVimMode(titleInput);
  const focusTitle = (mode: "normal" | "insert"): void => {
    const input = titleInput.current;
    if (!input) return;
    input.focus({ preventScroll: true });
    const end = input.value.length;
    if (mode === "normal" && inputVimField(input)) enterInputNormal(input, end);
    else input.setSelectionRange(end, end);
  };
  const bodyCaretOnFirstLine = (): boolean => {
    const handle = editor.current;
    if (!handle) return false;
    const { head } = handle.getSelection();
    return !handle.getValue().slice(0, head).includes("\n");
  };
  const localContent = (): DraftContent => {
    const body = textRef.current;
    const referenced = new Set(imageTokensInText(body).map((t) => t.id));
    return {
      body,
      attachments: attachmentsRef.current.filter((a) =>
        !a.pending && (!a.isImage || referenced.has(a.id))
      ),
    };
  };
  const retryReconcile = (): void => {
    clearTimeout(reconcileTimer.current);
    if (mounted.current) {
      reconcileTimer.current = setTimeout(() => reconcileRef.current(), 250);
    }
  };
  /** Fold newer document content into the open editor (Obsidian Sync style):
   * adopt it when nothing is typed since the shadow, otherwise merge both
   * three-way and let autosave write the result. */
  const reconcile = (): void => {
    if (writing.current) return;
    const document = owner.get().document;
    if (!document || document.kind !== "document") return;
    const remote = draftContent(document);
    if (sameDraftContent(remote, shadow.current)) return;
    if (encoding.current.size) {
      retryReconcile();
      return;
    }
    const kept = localContent();
    const merged = sameDraftContent(kept, shadow.current)
      ? remote
      : mergeDraftContent(
        shadow.current,
        { body: textRef.current, attachments: attachmentsRef.current },
        remote,
      );
    const next = merged ?? remote;
    if (next.body !== textRef.current) {
      const previous = textRef.current;
      textRef.current = next.body;
      if (!editor.current?.applyRemoteText(next.body)) {
        // An IME composition owns the editor; never write under it.
        textRef.current = previous;
        retryReconcile();
        return;
      }
      setText(next.body);
    }
    if (!merged) {
      // Too divergent to merge: keep this text as its own draft and show the
      // newer document here. Nothing is lost and nothing blocks typing.
      void draftRepository().create(
        `${titleRef.current.trim() || "Untitled"} (conflicted copy)`,
        document.parent_id,
        "document",
        kept.body,
        kept.attachments,
      ).then(() =>
        documentNotice(
          "Edited on two devices. Your version was kept as a copy.",
        )
      ).catch((e: Error) => documentNotice(e.message));
    }
    if (
      JSON.stringify(next.attachments) !==
        JSON.stringify(attachmentsRef.current)
    ) {
      seedInlineAttachments(next.attachments);
      attachmentsRef.current = next.attachments;
      setAttachments(next.attachments);
      editor.current?.refreshImages();
    }
    shadow.current = remote;
    if (!sameDraftContent(localContent(), remote)) schedule();
  };
  const reconcileRef = useRef(reconcile);
  reconcileRef.current = reconcile;
  const flush = async (): Promise<void> => {
    clearTimeout(timer.current);
    if (!dirtyRef.current && encoding.current.size === 0) return saving.current;
    await Promise.all(encoding.current);
    const nextTitle = titleRef.current.trim() || "Untitled";
    let body = "";
    const task = saving.current.catch(() => undefined).then(async () => {
      const saved = owner.get().document;
      if (!saved) throw new Error("Draft is unavailable");
      reconcile();
      const written = localContent();
      body = written.body;
      if (!sameDraftContent(written, shadow.current)) {
        writing.current = true;
        try {
          await owner.change(
            { type: "write", ...written },
            undefined,
            shadow.current,
          );
          shadow.current = written;
        } catch (cause) {
          if (!(cause instanceof DraftMergeError)) throw cause;
          // A closing editor cannot fold the newer document in first. Keep
          // this text as its own draft instead of dropping it.
          await draftRepository().create(
            `${nextTitle} (conflicted copy)`,
            saved.parent_id,
            "document",
            written.body,
            written.attachments,
          );
          documentNotice(
            "Edited on two devices. Your version was kept as a copy.",
          );
          shadow.current = written;
        } finally {
          writing.current = false;
        }
        reconcile();
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
      clearTimeout(reconcileTimer.current);
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
  useEffect(() => {
    reconcileRef.current();
  }, [current]);
  const attach = (files: File[]): void => {
    if (current.deleted) return;
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
          const next = [
            ...attachmentsRef.current.filter((a) => a.id !== attached.id),
            attached,
          ];
          // Refuse here what the server would refuse on every later save.
          if (!draftAttachmentsFit(next.filter((a) => !a.pending))) {
            throw new Error(
              `${
                file.name || "This file"
              } does not fit. A draft holds up to 100 attachments and 8 MB in total.`,
            );
          }
          attachmentsRef.current = next;
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
      // History is how a user recovers from a save problem; never gate it.
      await flush().catch((e: Error) => setError(e.message));
      const repository = draftRepository();
      const index = await repository.historyIndex(initial.id);
      setHistory(index);
      // The newest versions are the likeliest to be opened: fetch them
      // while the link has room, so opening one is instant.
      historyPrefetch.current?.abort();
      const prefetch = new AbortController();
      historyPrefetch.current = prefetch;
      void repository.prefetchHistory(
        initial.id,
        index.slice(0, 3).map((entry) => entry.revision),
        prefetch.signal,
      );
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Recovery unavailable");
    } finally {
      setHistoryLoading(false);
    }
  };
  const recoverCopy = async (): Promise<void> => {
    const copy = await draftRepository().create(
      `${titleRef.current} (recovered)`,
      // Beside the original, in its folder.
      current.parent_id,
      "document",
      textRef.current,
      attachmentsRef.current.filter((a) => !a.pending),
    );
    await owner.useRemote();
    dirtyRef.current = false;
    documentNotice("Your local version was preserved as a separate draft.");
    globalThis.location.hash = `drafts/${copy}`;
  };
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
      sx={{
        flex: 1,
        minWidth: 0,
        ...(focusLayout && {
          "& .MuiInputBase-input": {
            fontSize: "inherit",
            fontWeight: "inherit",
            lineHeight: "inherit",
          },
        }),
      }}
      inputRef={titleInput}
      disabled={current.deleted}
      inputProps={{
        "aria-label": "Draft title",
        maxLength: 160,
        enterKeyHint: desktop ? undefined : "next",
      }}
      onKeyDown={(e) => {
        if (isImeKeyEvent(e.nativeEvent)) return;
        if (e.metaKey || e.ctrlKey || e.altKey) return;
        if (!desktop) {
          if (
            e.key === "Enter" && !e.shiftKey &&
            !imeOwnsEditable(
              composing.current,
              compositionEndedAt.current,
              Date.now(),
            )
          ) {
            e.preventDefault();
            editor.current?.focusSelection({ anchor: 0, head: 0 });
          }
          return;
        }
        const toStart = e.key === "Enter" || e.key === "ArrowDown" ||
          (e.key === "Tab" && !e.shiftKey) ||
          (e.code === "KeyJ" && !e.shiftKey && isInputVimNormal(e.target));
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
              // The root rem already includes the reading preference. A second
              // multiplication made titles smaller than body text at 50%.
              fontSize: "1.75rem",
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
  // and rides above the keyboard. The keyboard toggle keeps its slot throughout
  // opening/closing; the formatting strip never changes width mid-gesture.
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
      <DraftFormatStrip sx={floatingMaterialSx}>
        {formatCommands.map(({ id, command }) => (
          <IconButton
            key={id}
            data-draft-tool
            aria-label={command.label}
            disabled={current.deleted && id !== "sourceMode"}
            {...keepEditorFocus}
            onClick={() => {
              if (editor.current && (!current.deleted || id === "sourceMode")) {
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
          disabled={current.deleted}
          {...keepEditorFocus}
          onClick={() => filePicker.current?.click()}
          sx={{ flexShrink: 0, width: 44, height: 44 }}
        >
          <AttachFile />
        </IconButton>
      </DraftFormatStrip>
      <DraftKeyboardToggle
        editor={editor}
        disabled={current.deleted}
        sx={{
          ...floatingMaterialSx,
          color: "text.primary",
          width: 48,
          height: 48,
          flexShrink: 0,
        }}
      />
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
                          disabled={current.deleted && id !== "sourceMode"}
                          onPointerDown={(e) => e.preventDefault()}
                          onMouseDown={(e) => e.preventDefault()}
                          onClick={() => {
                            if (
                              editor.current &&
                              (!current.deleted || id === "sourceMode")
                            ) {
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
            disabled={current.deleted}
            sx={desktop ? desktopDraftActionSx : undefined}
            {...keepEditorFocus}
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
            {...keepEditorFocus}
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
        compositionEndedAt.current = 0;
      }}
      onCompositionEndCapture={() => {
        composing.current = false;
        compositionEndedAt.current = Date.now();
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
                    exportDraft(
                      title,
                      textRef.current,
                      attachmentsRef.current,
                      !desktop,
                    )}
                >
                  <DownloadIcon />
                </IconButton>
                <IconButton
                  aria-label="Draft actions"
                  onClick={() => {
                    // The menu holds recovery actions too; a save problem
                    // must not make it unreachable.
                    void flush().catch((e: Error) => setError(e.message));
                    mobileChrome.onMenu();
                  }}
                >
                  <EllipsisIcon />
                </IconButton>
              </Stack>
            </Stack>
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
            {desktop && titleMode !== null && (
              // While the title owns focus it names the way back to the body
              // and the keys of its current mode (FOCUS.md "Draft document").
              <Box
                component="span"
                role="note"
                data-draft-title-hint={titleMode}
                sx={{
                  display: "inline-flex",
                  alignItems: "center",
                  flexShrink: 0,
                  gap: "0.25rem",
                  mr: "0.5rem",
                  color: "text.secondary",
                  typography: "caption",
                  whiteSpace: "nowrap",
                  "@container draft-editor (max-width: 28rem)": {
                    display: "none",
                  },
                }}
              >
                {(titleMode === "normal"
                  ? [["J", "body"], ["I", "edit"], ["Esc", "back"]]
                  : titleMode === "insert"
                  ? [["Esc", "normal"], ["↵", "body"]]
                  : [["↵", "body"], ["Esc", "back"]]).map(([key, label]) => (
                    <Box
                      key={key}
                      component="span"
                      sx={{
                        display: "inline-flex",
                        alignItems: "center",
                        mr: "0.375rem",
                      }}
                    >
                      <Kbd keys={key!} variant="context" />
                      <Box component="span" sx={{ ml: "0.25rem" }}>{label}</Box>
                    </Box>
                  ))}
              </Box>
            )}
            {desktop && (
              <HintTooltip
                title="Go to the title"
                shortcut={desktopWorkspaceSequence(DESKTOP_DOCUMENT_KEYS.title)}
                detail="Or k / ↑ on the first line"
              >
                <Box
                  component="span"
                  data-draft-title-shortcut
                  sx={{ display: "inline-flex", flexShrink: 0 }}
                >
                  <LeaderKeycap
                    leaderKey={DESKTOP_DOCUMENT_KEYS.title}
                  />
                </Box>
              </HintTooltip>
            )}
            {
              /* Desktop shows these actions once, in the bottom document bar
                with their leader slots; Mobile keeps them beside the title. */
            }
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
                      exportDraft(
                        title,
                        textRef.current,
                        attachmentsRef.current,
                        !desktop,
                      )}
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
          This draft is in Trash. Restore it to keep editing.
        </Alert>
      )}
      {(error || syncError) && (
        <Alert
          // Waiting for the server is not a failure: the text is saved here.
          severity={phase === "conflict"
            ? "warning"
            : !error && phase === "local"
            ? "info"
            : "error"}
          action={phase === "conflict"
            ? (
              <Button
                color="inherit"
                onClick={() =>
                  void recoverCopy().catch((e: Error) =>
                    setError(e.message)
                  )}
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
          {error ??
            (phase === "local"
              ? "Saved on this device. It will sync when the server is reachable."
              : syncError)}
        </Alert>
      )}
      <Box
        ref={documentScroll}
        data-draft-document-scroll={focusLayout ? "true" : undefined}
        sx={focusLayout
          ? {
            flex: 1,
            minHeight: 0,
            minWidth: 0,
            overflowY: "auto",
            overflowX: "hidden",
            overscrollBehaviorY: "contain",
            touchAction: "pan-y pinch-zoom",
            scrollPaddingBlock: "12px",
          }
          : { display: "contents" }}
      >
        <Stack
          sx={focusLayout
            ? { minHeight: "100%", minWidth: 0 }
            : { display: "contents" }}
        >
          {focusLayout && (
            <Box
              data-draft-inline-title
              sx={{ px: "18px", pt: 2, flexShrink: 0 }}
            >
              {titleField}
              <Typography variant="caption" color="text.disabled" role="status">
                {saveStatus}
              </Typography>
            </Box>
          )}
          <Box
            sx={{
              display: "flex",
              flex: focusLayout ? "1 0 auto" : 1,
              minHeight: focusLayout ? "auto" : 0,
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
              focusTitle(e.key === "ArrowUp" ? "insert" : "normal");
            }}
          >
            <Box
              ref={canvas}
              sx={{
                display: "flex",
                flexDirection: focusLayout ? "column" : undefined,
                flex: focusLayout ? "1 0 auto" : 1,
                minHeight: focusLayout ? "auto" : 0,
                minWidth: 0,
                width: "100%",
                "& [data-mobile-native-editor]": {
                  flex: focusLayout ? "1 0 auto" : 1,
                  display: "flex",
                  flexDirection: "column",
                  height: focusLayout ? "auto" : "100%",
                  maxHeight: "none",
                },
                "& textarea[data-mobile-native-textarea]": {
                  flex: focusLayout ? "1 0 auto" : 1,
                  ...(focusLayout
                    ? { overflowY: "hidden" }
                    : { height: "100% !important" }),
                  maxHeight: "none !important",
                },
                ...(focusLayout && {
                  // A Draft is a document, rather than a bounded chat input. One
                  // ancestor scrolls its inline title and body. CM6 can virtualize
                  // against that ancestor; native prose uses its existing fitter.
                  "& > [data-mobile-pager-ignore], & .cm-theme-none, & .cm-editor":
                    {
                      flex: "1 0 auto",
                      display: "flex",
                      flexDirection: "column",
                      maxHeight: "none !important",
                      height: "auto !important",
                    },
                  "& .cm-scroller": {
                    flex: "1 0 auto",
                    maxHeight: "none !important",
                    overflow: "visible !important",
                  },
                }),
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
                  // A merged remote update already set textRef; it is not typing.
                  const typed = value !== textRef.current;
                  textRef.current = value;
                  setText(value);
                  if (typed) schedule();
                }}
                {...(positions.has(initial.id)
                  ? { initialSelection: positions.get(initial.id)!.head }
                  : {})}
                onPasteFiles={attach}
                // In Trash the server refuses every write, and each refused
                // autosave would keep the text as one more conflicted copy.
                disabled={current.deleted}
                placeholder="Start writing…"
                borderless
                fill={!focusLayout}
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
                  onDelete={current.deleted ? undefined : () => {
                    attachmentsRef.current = attachmentsRef.current.filter((
                      item,
                    ) => item.id !== a.id);
                    setAttachments(attachmentsRef.current);
                    schedule();
                  }}
                />
              ))}
            </Stack>
          )}
        </Stack>
      </Box>
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
                exportDraft(
                  title,
                  textRef.current,
                  attachmentsRef.current,
                  !desktop,
                )}
              onTitle={() => focusTitle("normal")}
            />
          </Suspense>
        )
        : focusLayout
        ? focusToolbar
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
          onClose={closeHistory}
          actions={<Button onClick={closeHistory}>Close</Button>}
        >
          <Typography variant="body2" color="text.secondary">
            Snapshots taken every few minutes while you write, and the text as
            it was before each pause. Restoring creates a new version.
          </Typography>
          <Box sx={{ maxHeight: "50dvh", overflowY: "auto" }}>
            {history.length === 0 && (
              <Typography sx={{ py: 2 }}>No earlier versions yet.</Typography>
            )}
            {history.map((version) => (
              <Button
                key={version.revision}
                fullWidth
                data-draft-history-version={version.revision}
                aria-pressed={historyPreview?.summary.revision ===
                  version.revision}
                onClick={() => previewVersion(version)}
                sx={{
                  flexDirection: "column",
                  alignItems: "flex-start",
                  textTransform: "none",
                  textAlign: "left",
                }}
              >
                <span>
                  {new Date(version.updated_at_ms).toLocaleString()} ·{" "}
                  {version.characters.toLocaleString()} characters
                  {version.attachments > 0
                    ? ` · ${version.attachments.toLocaleString()} ${
                      version.attachments === 1 ? "attachment" : "attachments"
                    }`
                    : ""}
                </span>
                <Typography
                  variant="caption"
                  color="text.secondary"
                  noWrap
                  sx={{ maxWidth: "100%" }}
                >
                  {version.preview || "Empty"}
                </Typography>
              </Button>
            ))}
            {historyPreview && (
              <>
                <Box
                  component="pre"
                  data-draft-history-preview
                  sx={{
                    whiteSpace: "pre-wrap",
                    overflowWrap: "anywhere",
                    fontSize: "0.9rem",
                  }}
                >
                  {historyPreview.document?.body ??
                    historyPreview.error ??
                    "Loading this version…"}
                </Box>
                <Button
                  disabled={!historyPreview.document}
                  onClick={() => {
                    const version = historyPreview.document;
                    if (!version) return;
                    void owner.change({
                      type: "write",
                      body: version.body,
                      attachments: version.attachments,
                    }).then(() => {
                      dirtyRef.current = false;
                      closeHistory();
                      onReload();
                    }).catch((e: Error) => setError(e.message));
                  }}
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
