import { CheckCircle, OpenInNew } from "@mui/icons-material";
import { Alert, Box, Button, IconButton, List, ListItemButton, ListItemText, Tooltip, Typography } from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import { desktopSize } from "../../surface/desktopSize";
import { mutateMobileReview } from "../../store";
import { DESKTOP_SURFACE_RADIUS } from "../DesktopEmbeddedControl";
import { extensionRequest, type ExtensionResponse } from "../../extensions/api";
import { readRemoteReview } from "../../mobile/review/RemoteReviewApp";
import {
  remoteFileKey,
  remoteFileMark,
  type RemoteReviewBinding,
  type RemoteReviewFile,
  type RemoteReviewPage,
  sameReview,
} from "../../mobile/review/remoteReviewModel";
import { type Choice, choices, matches } from "../../mobile/review/remoteReviewSources";
import { rememberPullFiles } from "./pullPatches";
import { ReviewPullPicker } from "./ReviewPullPicker";
import type { DesktopReviewContext } from "./useDesktopReviewContext";

type Inventory = Extract<ExtensionResponse, { type: "inventory" }>;

function repositoryOf(choice: Choice, binding: RemoteReviewBinding): string | undefined {
  return choice.extension.views.some((view) => view.id === choice.view && view.discovery)
    ? `${binding.owner}/${binding.repository}`
    : undefined;
}

/** The bound PR: its facts and its files, a page of 20 at a time. */
function PullFiles({
  context,
  choice,
  binding,
  progress,
  openPath,
  onOpen,
  onChange,
}: {
  context: string;
  choice: Choice;
  binding: RemoteReviewBinding;
  progress: Readonly<Record<string, string>>;
  openPath: string | undefined;
  onOpen: (binding: RemoteReviewBinding, revision: string, file: RemoteReviewFile) => void;
  onChange: () => void;
}): React.JSX.Element {
  const [page, setPage] = useState<{ number: number; review: RemoteReviewPage }>();
  const [error, setError] = useState<string>();
  const [request, setRequest] = useState(1);
  useEffect(() => {
    const controller = new AbortController();
    setError(undefined);
    const previous = request > 1 ? page?.review : undefined;
    void readRemoteReview(context, choice, binding.number, request, controller.signal, binding.repositoryId,
      previous?.revision, repositoryOf(choice, binding)).then((review) => {
      if (controller.signal.aborted) return;
      if (previous && !sameReview(previous, review)) throw new Error("This PR changed. Refresh before reading another page.");
      rememberPullFiles(binding, review.revision, review.files);
      setPage({ number: request, review });
    }).catch((reason: unknown) => {
      if (!controller.signal.aborted) setError(reason instanceof Error ? reason.message : "Could not read the pull request.");
    });
    return () => controller.abort();
    // `page` is the previous observation this request continues.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [binding, choice, context, request]);
  const review = page?.review;
  const viewed = (file: RemoteReviewFile): boolean => {
    const key = remoteFileKey(binding, file.path);
    return !!key && progress[key] === remoteFileMark(file);
  };
  return (
    <Box sx={{ display: "flex", flexDirection: "column", minHeight: 0, flex: 1 }} data-review-pull-files>
      <Box sx={{ px: 1.5, py: 0.5, display: "flex", alignItems: "center", gap: 0.5 }}>
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography noWrap sx={{ fontSize: "0.8rem", fontWeight: 650 }} title={review?.title}>
            {review?.title ?? `#${binding.number}`}
          </Typography>
          <Typography noWrap sx={{ fontSize: "0.68rem", color: "text.secondary" }} data-review-pull-facts>
            {`${binding.owner}/${binding.repository}#${binding.number}`}
            {review && ` · ${review.state} · ${review.head.slice(0, 8)} · ${String(review.totalFiles)} files · ${String(review.files.filter(viewed).length)}/${String(review.files.length)} viewed`}
          </Typography>
        </Box>
        {review && (
          <Tooltip title="Open on GitHub">
            <IconButton size="small" component="a" href={review.url} target="_blank" rel="noreferrer" aria-label="Open the pull request on GitHub">
              <OpenInNew fontSize="small" />
            </IconButton>
          </Tooltip>
        )}
        <Button size="small" onClick={onChange} data-review-pull-change sx={{ textTransform: "none", fontSize: "0.72rem" }}>Change</Button>
      </Box>
      {error && <Alert severity="warning" sx={{ mx: 1, py: 0 }} action={<Button size="small" onClick={() => setRequest(1)}>Refresh</Button>}>{error}</Alert>}
      {review?.limited && <Alert severity="info" sx={{ mx: 1, py: 0 }}>Past the 3,000-file preview limit; open GitHub for the rest.</Alert>}
      {!review && !error && <Typography sx={{ p: 2, fontSize: "0.8rem", color: "text.secondary" }}>Reading the pull request…</Typography>}
      <List dense disablePadding sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: 0.5 }}>
        {review?.files.map((file) => (
          <ListItemButton
            key={file.path}
            data-desktop-item={file.path}
            data-review-pull-file={file.path}
            selected={file.path === openPath}
            onClick={() => onOpen(binding, review.revision, file)}
            sx={{ py: 0.25, px: 1, gap: 1, minHeight: desktopSize(30), borderRadius: `${String(DESKTOP_SURFACE_RADIUS)}px`, opacity: viewed(file) ? 0.62 : 1 }}
          >
            <ListItemText
              primary={file.path}
              secondary={file.oldPath ? `from ${file.oldPath}` : file.status}
              slotProps={{ primary: { noWrap: true, sx: { fontSize: "0.78rem", direction: "rtl", textAlign: "left" } }, secondary: { sx: { fontSize: "0.68rem" } } }}
              sx={{ my: 0, minWidth: 0 }}
            />
            <Typography component="span" sx={{ flexShrink: 0, fontFamily: "monospace", fontSize: "0.7rem" }}>
              <Box component="span" sx={{ color: "success.main" }}>+{file.additions}</Box>{" "}
              <Box component="span" sx={{ color: "error.main" }}>−{file.deletions}</Box>
            </Typography>
            {viewed(file) && <CheckCircle aria-label="Viewed" sx={{ fontSize: "1rem", color: "success.main", flexShrink: 0 }} />}
          </ListItemButton>
        ))}
        {review?.nextPage && (
          <Button data-desktop-item="pull-next" size="small" onClick={() => setRequest(review.nextPage!)} sx={{ m: 1 }}>
            Next files (page {review.nextPage})
          </Button>
        )}
      </List>
    </Box>
  );
}

/**
 * Pull request (`␣VG`): the Session's bound remote PR, or a picker to bind
 * one. Read-only; patches stay in memory and never reach synced state.
 */
export function ReviewPullList({
  context,
  openPath,
  onOpen,
}: {
  context: DesktopReviewContext;
  openPath: string | undefined;
  onOpen: (binding: RemoteReviewBinding, revision: string, file: RemoteReviewFile) => void;
}): React.JSX.Element {
  const code = context.target?.sessionId ?? "";
  const [inventory, setInventory] = useState<Inventory>();
  const [error, setError] = useState<string>();
  const [picking, setPicking] = useState(false);
  useEffect(() => {
    if (!code) return undefined;
    const controller = new AbortController();
    setInventory(undefined);
    setError(undefined);
    void extensionRequest(code, null, controller.signal).then((response) => {
      if (controller.signal.aborted) return;
      if (response.type !== "inventory") throw new Error("Extension inventory unavailable");
      setInventory(response);
    }).catch((reason: unknown) => {
      if (!controller.signal.aborted) setError(reason instanceof Error ? reason.message : "Extensions unavailable");
    });
    return () => controller.abort();
  }, [code]);
  const available = useMemo(() => choices(inventory), [inventory]);
  const binding = context.synced.remote_review ?? undefined;
  const bound = binding ? available.find((choice) => matches(choice, binding)) : undefined;
  const bind = async (choice: Choice, number: string): Promise<void> => {
    const review = await readRemoteReview(code, choice, number, 1, new AbortController().signal, undefined, undefined,
      `${choice.remote.owner}/${choice.remote.repository}`);
    const next: RemoteReviewBinding = {
      pluginId: choice.extension.identity.pluginId,
      view: choice.view,
      host: choice.remote.host,
      owner: choice.remote.owner,
      repository: choice.remote.repository,
      repositoryId: review.repositoryId,
      number,
    };
    rememberPullFiles(next, review.revision, review.files);
    if (context.stateKey) mutateMobileReview(context.stateKey, "setRemoteReview", { binding: next });
    setPicking(false);
  };
  if (error) return <Alert severity="warning" sx={{ m: 1 }}>{error}</Alert>;
  if (!inventory) return <Typography sx={{ p: 2, fontSize: "0.8rem", color: "text.secondary" }}>Reading extensions…</Typography>;
  if (available.length === 0) {
    return (
      <Typography sx={{ p: 2, fontSize: "0.8rem", color: "text.secondary" }} data-review-pull-unavailable>
        Connect GitHub on this session's Machine and configure a repository remote to review pull requests.
      </Typography>
    );
  }
  if (binding && bound && !picking) {
    return (
      <PullFiles
        context={code}
        choice={bound}
        binding={binding}
        progress={context.synced.progress}
        openPath={openPath}
        onOpen={onOpen}
        onChange={() => setPicking(true)}
      />
    );
  }
  return (
    <>
      {binding && !bound && (
        <Alert severity="warning" sx={{ mx: 1, mt: 0.5, py: 0 }}>
          The bound repository or extension is unavailable. Choose another pull request.
        </Alert>
      )}
      <ReviewPullPicker context={code} available={available} onSelect={bind} />
    </>
  );
}
