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
import { type MutableRefObject, useEffect, useRef, useState } from "react";
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
import { Sheet } from "../Sheet";
import { useBootReady } from "../useBootReady";
import { draftRepository, useDraftDocument } from "./store";
import { documentNotice } from "./DocumentNotifications";
import { type DraftDocument } from "./model";

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
export type DraftFlush = () => Promise<void>;

export function DraftEditor({ id, beforeLeave, onCopyToSession }: {
  id: string;
  beforeLeave: MutableRefObject<DraftFlush>;
  onCopyToSession: () => void;
}): React.JSX.Element {
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
    onReload,
  }: {
    initial: DraftDocument;
    current: DraftDocument;
    phase: string;
    syncError: string | null;
    beforeLeave: MutableRefObject<DraftFlush>;
    onCopyToSession: () => void;
    onReload: () => void;
  },
): React.JSX.Element {
  const editor = useRef<ComposerEditorHandle | null>(null);
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
  const vim = useVimSetting();
  const toolbar = useComposerToolbar();
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
  return (
    <Stack
      sx={{ flex: 1, minHeight: 0, minWidth: 0 }}
      data-draft-editor={initial.id}
      onCompositionStartCapture={() => {
        composing.current = true;
      }}
      onCompositionEndCapture={() => {
        composing.current = false;
      }}
    >
      <Stack
        direction="row"
        alignItems="center"
        spacing={desktop ? "0.25rem" : 1}
        sx={{
          ...(desktop ? { "& .MuiIconButton-root": desktopDraftActionSx } : {}),
          px: desktop ? "1rem" : 1,
          py: 0.5,
          borderBottom: 1,
          borderColor: "divider",
        }}
      >
        <TextField
          variant="standard"
          value={title}
          placeholder="Untitled"
          fullWidth
          sx={{ flex: 1, minWidth: 0 }}
          inputProps={{ "aria-label": "Draft title", maxLength: 160 }}
          onChange={(e) => {
            setTitle(e.target.value);
            titleRef.current = e.target.value;
            schedule();
          }}
          onBlur={() => void flush().catch((e: Error) => setError(e.message))}
          slotProps={{
            input: {
              disableUnderline: true,
              sx: { fontSize: "1.4rem", fontWeight: 600 },
            },
          }}
        />
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
      </Stack>
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
        }}
        data-desktop-region={desktop ? "prompt.composer" : undefined}
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
          pb: desktop ? 0.5 : "max(4px, env(safe-area-inset-bottom))",
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
