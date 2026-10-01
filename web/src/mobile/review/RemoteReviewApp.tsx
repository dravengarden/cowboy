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
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { lazy, Suspense, useEffect, useRef, useState } from "react";
import {
  extensionRequest,
  type ExtensionResponse,
  type RepositoryRemote,
  resourceQuery,
  type WorkspaceExtension,
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

const CodeViewer = lazy(() => import("./CodeViewer"));
type Inventory = Extract<ExtensionResponse, { type: "inventory" }>;
type Choice = {
  extension: WorkspaceExtension;
  view: string;
  remote: RepositoryRemote;
  key: string;
};

function choices(inventory: Inventory | undefined): Choice[] {
  return inventory?.extensions.filter((extension) => extension.available)
    .flatMap((extension) =>
      extension.views.filter((view) => view.review === "pull_request").flatMap((
        view,
      ) =>
        inventory.remotes.map((remote) => ({
          extension,
          view: view.id,
          remote,
          key: JSON.stringify([
            extension.identity.pluginId,
            view.id,
            remote.name,
          ]),
        }))
      )
    ) ?? [];
}

function matches(choice: Choice, binding: RemoteReviewBinding): boolean {
  return choice.extension.identity.pluginId === binding.pluginId &&
    choice.view === binding.view &&
    choice.remote.host === binding.host &&
    choice.remote.owner === binding.owner &&
    choice.remote.repository === binding.repository;
}

async function read(
  context: string,
  choice: Choice,
  number: string,
  page: number,
  signal: AbortSignal,
  repositoryId?: string,
  revision?: string,
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
    pullNumber(url.href, choice.remote) !== number
  ) throw new Error("The PR repository changed.");
  return response.review;
}

function PullRequest({ context, choice, binding, initial, active }: {
  context: string;
  choice: Choice;
  binding: RemoteReviewBinding;
  initial: RemoteReviewPage | undefined;
  active: boolean;
}): React.JSX.Element {
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
        {file && (
          <IconButton
            aria-label="PR files"
            onClick={() => setSelected(undefined)}
          >
            <ArrowBack />
          </IconButton>
        )}
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
      {error && <Alert severity="warning">{error}</Alert>}
      {loading && (
        <Box role="status" sx={{ p: 1 }}>
          <CircularProgress size={20} />
        </Box>
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
  const [selection, setSelection] = useState("");
  const [input, setInput] = useState(binding?.number ?? "");
  const [loading, setLoading] = useState(false);
  const [seed, setSeed] = useState<{ key: string; review: RemoteReviewPage }>();
  const request = useRef<AbortController | undefined>(undefined);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(false);
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
    return () => {
      controller.abort();
      request.current?.abort();
    };
  }, [context, refresh]);
  const available = choices(inventory);
  const boundChoice = binding
    ? available.find((choice) => matches(choice, binding))
    : undefined;
  const selectedChoice = available.find((choice) => choice.key === selection) ??
    boundChoice ?? available[0];
  async function associate() {
    if (!selectedChoice) return;
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    setError(undefined);
    setLoading(true);
    try {
      const number = pullNumber(input, selectedChoice.remote);
      const review = await read(
        context,
        selectedChoice,
        number,
        1,
        controller.signal,
      );
      if (controller.signal.aborted) return;
      const binding: RemoteReviewBinding = {
        pluginId: selectedChoice.extension.identity.pluginId,
        view: selectedChoice.view,
        host: selectedChoice.remote.host,
        owner: selectedChoice.remote.owner,
        repository: selectedChoice.remote.repository,
        repositoryId: review.repositoryId,
        number,
      };
      setSeed({ key: JSON.stringify(binding), review });
      onBind(binding);
      setEditing(false);
    } catch (error) {
      if (!controller.signal.aborted) {
        setError(
          error instanceof Error ? error.message : "Could not associate PR",
        );
      }
    } finally {
      if (!controller.signal.aborted) setLoading(false);
    }
  }
  return (
    <Stack
      sx={{
        height: "100%",
        minWidth: 0,
        bgcolor: "background.default",
        pt: "var(--cowboy-system-top-clearance)",
      }}
    >
      <Stack
        direction="row"
        alignItems="center"
        sx={{ minHeight: 52, px: 1, borderBottom: 1, borderColor: "divider" }}
      >
        <IconButton aria-label="Local worktree review" onClick={onLocal}>
          <ArrowBack />
        </IconButton>
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography variant="body2" noWrap>
            {binding
              ? `${binding.owner}/${binding.repository} #${binding.number}`
              : "Remote PR"}
          </Typography>
          <Typography variant="caption" color="text.secondary" noWrap>
            {title}
          </Typography>
        </Box>
        {binding && (
          <Button
            onClick={() => {
              request.current?.abort();
              setLoading(false);
              setEditing((value) => !value);
            }}
          >
            {editing ? "Cancel" : "Change PR"}
          </Button>
        )}
        <IconButton
          aria-label="Refresh extensions"
          onClick={() => setRefresh((n) => n + 1)}
        >
          <Refresh />
        </IconButton>
        <WorkspaceExtensionsButton context={context} />
      </Stack>
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
      {!inventory && !error && <CircularProgress size={24} sx={{ m: 2 }} />}
      {inventory && !available.length && (
        <Alert severity="info">
          Install a compatible GitHub extension on this session’s Machine and
          configure a repository remote. Then refresh Extensions.
        </Alert>
      )}
      {inventory && (editing || !binding)
        ? (
          <Stack spacing={2} sx={{ p: 2, ...mobileNativeYScrollSx }}>
            <Typography>
              Associate this session with a remote pull request.
            </Typography>
            <TextField
              select
              label="Repository"
              value={selectedChoice?.key ?? ""}
              disabled={loading || !available.length}
              onChange={(e) => {
                setSelection(e.target.value);
                setError(undefined);
              }}
            >
              {available.map((choice) => (
                <MenuItem key={choice.key} value={choice.key}>
                  {choice.remote.owner}/{choice.remote.repository} ·{" "}
                  {choice.remote.name} · {choice.extension.label}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              label="PR URL or number"
              value={input}
              disabled={loading}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !loading) void associate();
              }}
            />
            <Button
              variant="contained"
              disableElevation
              disabled={loading || !selectedChoice || !input.trim()}
              onClick={() => void associate()}
            >
              {loading ? "Reading PR…" : "Associate and review"}
            </Button>
            {binding && (
              <Button
                color="error"
                disabled={loading}
                onClick={() => {
                  onBind(null);
                  setSeed(undefined);
                  setEditing(true);
                }}
              >
                Remove association
              </Button>
            )}
          </Stack>
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
          />
        )
        : inventory && binding && (
          <Alert severity="warning">
            The associated repository or extension is unavailable. Choose Change
            PR or restore the Machine connection.
          </Alert>
        )}
    </Stack>
  );
}
