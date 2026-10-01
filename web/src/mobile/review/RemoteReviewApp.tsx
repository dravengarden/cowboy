import {
  ArrowBack,
  ChevronLeft,
  ChevronRight,
  OpenInNew,
  Refresh,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  IconButton,
  List,
  ListItemButton,
  ListItemText,
  Stack,
  Typography,
} from "@mui/material";
import { lazy, Suspense, useEffect, useRef, useState } from "react";
import {
  extensionRequest,
  type ExtensionResponse,
  resourceQuery,
} from "../../extensions/api";
import { mobileNativeYScrollSx } from "../../mobileNativeOverflow";
import {
  pullNumber,
  type RemoteReviewBinding,
  type RemoteReviewPage,
  sameReview,
} from "./remoteReviewModel";
import { useReviewSettings } from "./reviewSettings";
import { WorkspaceExtensionsButton } from "../../extensions/WorkspaceExtensionsButton";

import { type Choice, choices, matches } from "./remoteReviewSources";
import {
  PullSkeleton,
  remoteBottomSx,
  RemotePullPicker,
} from "./RemotePullPicker";

const CodeViewer = lazy(() => import("./CodeViewer"));
type Inventory = Extract<ExtensionResponse, { type: "inventory" }>;
async function read(
  context: string,
  choice: Choice,
  number: string,
  page: number,
  signal: AbortSignal,
  repositoryId?: string,
  revision?: string,
  repository?: string,
): Promise<RemoteReviewPage> {
  const query = resourceQuery(
    choice.extension.identity,
    choice.remote.name,
    choice.view,
    "",
    page,
    number,
  );
  query.set("review", "true");
  if (repository) query.set("repository", repository);
  if (repositoryId) query.set("repositoryId", repositoryId);
  if (revision) query.set("revision", revision);
  const response = await extensionRequest(context, query, signal);
  if (
    response.type !== "review" || response.review.number !== number ||
    (repositoryId && response.review.repositoryId !== repositoryId) ||
    (revision && response.review.revision !== revision)
  ) throw new Error("The PR identity changed. Refresh or associate it again.");
  const url = new URL(response.review.url);
  if (
    url.hostname !== choice.remote.host ||
    pullNumber(
        url.href,
        repository
          ? {
            ...choice.remote,
            owner: repository.split("/")[0]!,
            repository: repository.split("/")[1]!,
          }
          : choice.remote,
      ) !== number
  ) throw new Error("The PR repository changed.");
  return response.review;
}

function PullRequest(
  { context, choice, binding, initial, active, onLocal, onChoose }: {
    context: string;
    choice: Choice;
    binding: RemoteReviewBinding;
    initial: RemoteReviewPage | undefined;
    active: boolean;
    onLocal: () => void;
    onChoose: () => void;
  },
): React.JSX.Element {
  const [snapshot, setSnapshot] = useState(initial);
  const [page, setPage] = useState(1);
  const [selected, setSelected] = useState<string>();
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(false);
  const controller = useRef<AbortController | undefined>(undefined);
  const observed = useRef(snapshot);
  observed.current = snapshot;
  const positions = useRef(new Map<string, number>());
  const settings = useReviewSettings();
  const requestKey = JSON.stringify([
    context,
    choice.key,
    choice.extension.identity,
    binding,
  ]);
  async function load(next: number, previous?: RemoteReviewPage) {
    controller.current?.abort();
    const request = new AbortController();
    controller.current = request;
    setLoading(true);
    setError(undefined);
    try {
      const value = await read(
        context,
        choice,
        binding.number,
        next,
        request.signal,
        binding.repositoryId,
        previous?.revision,
        choice.extension.views.some((view) =>
            view.id === choice.view && view.discovery
          )
          ? `${binding.owner}/${binding.repository}`
          : undefined,
      );
      if (request.signal.aborted) return;
      if (previous && !sameReview(previous, value)) {
        throw new Error(
          "This PR changed. Refresh before reading another page.",
        );
      }
      setSnapshot(value);
      setPage(next);
      setSelected(undefined);
    } catch (error) {
      if (!request.signal.aborted) {
        setError(error instanceof Error ? error.message : "Could not read PR");
      }
    } finally {
      if (!request.signal.aborted) setLoading(false);
    }
  }
  const loadRef = useRef(load);
  loadRef.current = load;
  useEffect(() => {
    if (active && !observed.current) void loadRef.current(1);
    if (!active) setLoading(false);
    return () => controller.current?.abort();
  }, [requestKey, active, initial]);
  const file = snapshot?.files.find((file) => file.path === selected);
  const scrollKey = JSON.stringify([
    context,
    binding.repositoryId,
    binding.number,
    snapshot?.revision,
    selected,
  ]);
  return (
    <Stack sx={{ flex: 1, minHeight: 0 }}>
      <Stack direction="row" alignItems="center" sx={{ px: 1, flexShrink: 0 }}>
        <Box sx={{ flex: 1, minWidth: 0, py: 1 }}>
          <Typography variant="body2" noWrap>
            {file?.path ?? snapshot?.title ?? `PR #${binding.number}`}
          </Typography>
          <Typography variant="caption" color="text.secondary" noWrap>
            {snapshot
              ? `${snapshot.state} · ${
                snapshot.head.slice(0, 8)
              } · ${snapshot.totalFiles} files`
              : "Remote pull request"}
          </Typography>
        </Box>
      </Stack>
      {error && <Alert severity="warning">{error}</Alert>}
      {loading && (
        <Typography role="status" variant="caption" sx={{ px: 2, py: 1 }}>
          {snapshot ? "Updating PR…" : "Loading changed files…"}
        </Typography>
      )}
      {snapshot?.limited && (
        <Alert severity="info">
          This PR exceeds the 3,000-file preview limit. Open GitHub for the
          remaining files.
        </Alert>
      )}
      {file
        ? (
          <>
            {file.oldPath && (
              <Typography variant="caption" sx={{ px: 2 }}>
                Renamed from {file.oldPath}
              </Typography>
            )}
            {file.limited && (
              <Alert severity="info">
                GitHub did not provide a complete text patch for this file.
              </Alert>
            )}
            {file.patch
              ? (
                <Suspense
                  fallback={<CircularProgress size={24} sx={{ m: 2 }} />}
                >
                  <CodeViewer
                    key={scrollKey}
                    text={file.patch}
                    kind="diff"
                    path={file.path}
                    softWrap={settings.softWrap}
                    fontSize={settings.codeFontSize}
                    diagnostics={false}
                    inlayHints={false}
                    semanticHighlighting={false}
                    scrollRestoreKey={scrollKey}
                    savedScrollTop={positions.current.get(scrollKey)}
                    onScrollTopChange={(key, top) => {
                      if (
                        positions.current.size >= 40 &&
                        !positions.current.has(key)
                      ) {
                        positions.current.delete(
                          positions.current.keys().next().value!,
                        );
                      }
                      positions.current.set(key, top);
                    }}
                  />
                </Suspense>
              )
              : (
                <Typography sx={{ p: 2 }}>
                  No text patch is available. This may be a binary file or a
                  rename without text changes.
                </Typography>
              )}
          </>
        )
        : (
          <>
            <List sx={{ ...mobileNativeYScrollSx, flex: 1, minHeight: 0 }}>
              {snapshot?.files.map((file) => (
                <ListItemButton
                  key={file.path}
                  onClick={() => setSelected(file.path)}
                >
                  <ListItemText
                    primary={file.path}
                    secondary={`${file.status} · +${file.additions} −${file.deletions}${
                      file.limited ? " · preview limited" : ""
                    }`}
                    slotProps={{
                      primary: { sx: { overflowWrap: "anywhere" } },
                    }}
                  />
                  <ChevronRight />
                </ListItemButton>
              ))}
              {snapshot?.totalFiles === 0 && (
                <Typography sx={{ p: 2 }}>
                  This PR has no changed files.
                </Typography>
              )}
            </List>
            {snapshot && (
              <Stack
                direction="row"
                alignItems="center"
                justifyContent="space-between"
                sx={{ p: 1 }}
              >
                <IconButton
                  aria-label="Previous PR files"
                  disabled={loading || page === 1}
                  onClick={() => void load(page - 1, snapshot)}
                >
                  <ChevronLeft />
                </IconButton>
                <Typography variant="caption">
                  Page {page} · remote PR
                </Typography>
                <IconButton
                  aria-label="Next PR files"
                  disabled={loading || !snapshot.nextPage}
                  onClick={() =>
                    snapshot.nextPage && void load(snapshot.nextPage, snapshot)}
                >
                  <ChevronRight />
                </IconButton>
              </Stack>
            )}
          </>
        )}
      <Stack direction="row" alignItems="center" sx={remoteBottomSx}>
        <Button
          startIcon={<ArrowBack />}
          aria-label={file ? "PR files" : "Choose another PR"}
          onClick={() => file ? setSelected(undefined) : onChoose()}
          sx={{ minHeight: 44 }}
        >
          {file ? "Files" : "PRs"}
        </Button>
        <Button
          aria-label="Local worktree review"
          onClick={onLocal}
          sx={{ minHeight: 44 }}
        >
          Local
        </Button>
        <Box sx={{ flex: 1 }} />
        <IconButton
          aria-label="Refresh PR"
          disabled={loading}
          onClick={() => void load(1)}
        >
          <Refresh />
        </IconButton>
        <IconButton
          component="a"
          href={`https://${binding.host}/${binding.owner}/${binding.repository}/pull/${binding.number}`}
          target="_blank"
          rel="noopener noreferrer"
          aria-label="Open PR on GitHub"
        >
          <OpenInNew />
        </IconButton>
      </Stack>
    </Stack>
  );
}

export function RemoteReviewApp(
  { context, title, binding, active, onBind, onLocal }: {
    context: string;
    title: string;
    binding: RemoteReviewBinding | null | undefined;
    active: boolean;
    onBind: (binding: RemoteReviewBinding | null) => void;
    onLocal: () => void;
  },
): React.JSX.Element {
  const [inventory, setInventory] = useState<Inventory>();
  const [error, setError] = useState<string>();
  const [refresh, setRefresh] = useState(0);
  const [editing, setEditing] = useState(!binding);
  const [seed, setSeed] = useState<{ key: string; review: RemoteReviewPage }>();
  useEffect(() => {
    const controller = new AbortController();
    setError(undefined);
    setInventory(undefined);
    void extensionRequest(context, null, controller.signal).then((response) => {
      if (controller.signal.aborted) return;
      if (response.type !== "inventory") {
        throw new Error("Extension inventory unavailable");
      }
      setInventory(response);
    }).catch((error: unknown) => {
      if (!controller.signal.aborted) {
        setError(
          error instanceof Error ? error.message : "Extensions unavailable",
        );
      }
    });
    return () => controller.abort();
  }, [context, refresh]);
  const available = choices(inventory);
  const boundChoice = binding
    ? available.find((choice) => matches(choice, binding))
    : undefined;
  async function associate(
    choice: Choice,
    number: string,
    signal: AbortSignal,
  ) {
    const review = await read(
      context,
      choice,
      number,
      1,
      signal,
      undefined,
      undefined,
      choice.extension.views.some((view) =>
          view.id === choice.view && view.discovery
        )
        ? `${choice.remote.owner}/${choice.remote.repository}`
        : undefined,
    );
    if (signal.aborted) return;
    const binding: RemoteReviewBinding = {
      pluginId: choice.extension.identity.pluginId,
      view: choice.view,
      host: choice.remote.host,
      owner: choice.remote.owner,
      repository: choice.remote.repository,
      repositoryId: review.repositoryId,
      number,
    };
    setSeed({ key: JSON.stringify(binding), review });
    onBind(binding);
    setEditing(false);
  }
  const picking = editing || !binding;
  return (
    <Stack
      sx={{
        height: "100%",
        minHeight: 0,
        minWidth: 0,
        bgcolor: "background.default",
        pt: "var(--cowboy-system-top-clearance)",
      }}
    >
      <Box
        sx={{
          px: 2,
          py: 1.5,
          borderBottom: 1,
          borderColor: "divider",
          flexShrink: 0,
        }}
      >
        <Typography fontWeight={600}>
          {picking
            ? "Choose a pull request"
            : `${binding.owner}/${binding.repository} #${binding.number}`}
        </Typography>
        <Typography variant="caption" color="text.secondary" noWrap>
          {title}
        </Typography>
      </Box>
      {error && (
        <Alert
          severity="warning"
          action={
            <Button onClick={() => setRefresh((n) => n + 1)}>Retry</Button>
          }
        >
          {error}
        </Alert>
      )}
      {!inventory && !error && (
        <Box sx={{ flex: 1 }}>
          <PullSkeleton />
        </Box>
      )}
      {inventory && !available.length && (
        <Alert severity="info">
          Connect GitHub on this session’s Machine and configure a repository
          remote to browse pull requests.
        </Alert>
      )}
      {active && inventory && available.length > 0 && picking
        ? (
          <RemotePullPicker
            key={context}
            context={context}
            available={available}
            bound={!!binding}
            onSelect={associate}
            onReload={() => setRefresh((n) => n + 1)}
            onBack={() => binding ? setEditing(false) : onLocal()}
            onRemove={() => {
              onBind(null);
              setSeed(undefined);
              setEditing(true);
            }}
          />
        )
        : binding && boundChoice
        ? (
          <PullRequest
            key={JSON.stringify([
              context,
              binding,
              boundChoice.extension.identity,
            ])}
            context={context}
            choice={boundChoice}
            binding={binding}
            initial={seed?.key === JSON.stringify(binding)
              ? seed.review
              : undefined}
            active={active}
            onLocal={onLocal}
            onChoose={() => setEditing(true)}
          />
        )
        : inventory && binding && !picking && (
          <Alert severity="warning">
            The associated repository or extension is unavailable. Restore the
            Machine connection or choose another PR.
          </Alert>
        )}
      {(!inventory || !available.length || (!picking && !boundChoice)) && (
        <Stack direction="row" sx={{ ...remoteBottomSx, mt: "auto" }}>
          <Button
            startIcon={<ArrowBack />}
            aria-label="Local worktree review"
            onClick={onLocal}
          >
            Back
          </Button>
          {binding && (
            <Button onClick={() => setEditing(true)}>Change PR</Button>
          )}
          <Box sx={{ flex: 1 }} />
          <IconButton
            aria-label="Refresh extensions"
            onClick={() =>
              setRefresh((n) =>
                n + 1
              )}
          >
            <Refresh />
          </IconButton>
          <WorkspaceExtensionsButton context={context} />
        </Stack>
      )}
    </Stack>
  );
}
