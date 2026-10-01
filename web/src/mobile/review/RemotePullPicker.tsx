import {
  ArrowBack,
  ChevronRight,
  FilterList,
  Link,
  Refresh,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  IconButton,
  List,
  ListItemButton,
  MenuItem,
  Skeleton,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useRef, useState } from "react";
import {
  extensionRequest,
  type PullPage,
  type PullSummary,
  resourceQuery,
} from "../../extensions/api";
import { WorkspaceExtensionsButton } from "../../extensions/WorkspaceExtensionsButton";
import { mobileNativeYScrollSx } from "../../mobileNativeOverflow";
import { pullNumber } from "./remoteReviewModel";
import type { Choice } from "./remoteReviewSources";

export const remoteBottomSx = {
  flexShrink: 0,
  borderTop: 1,
  borderColor: "divider",
  px: 1,
  pt: 1,
  pb: "max(8px, env(safe-area-inset-bottom))",
  bgcolor: "background.default",
  "& .MuiIconButton-root": { width: 44, height: 44 },
};
export function PullSkeleton(): React.JSX.Element {
  return (
    <Stack
      role="status"
      aria-label="Loading pull requests"
      spacing={2}
      sx={{ p: 2 }}
    >
      {[0, 1, 2, 3].map((n) => (
        <Box key={n} sx={{ py: 1 }}>
          <Skeleton animation={false} width="45%" height={18} />
          <Skeleton animation={false} width="90%" height={28} />
          <Skeleton animation={false} width="65%" height={18} />
        </Box>
      ))}
    </Stack>
  );
}

export function RemotePullPicker(
  { context, available, onSelect, onBack, bound, onRemove, onReload }: {
    context: string;
    available: Choice[];
    bound: boolean;
    onSelect: (
      choice: Choice,
      number: string,
      signal: AbortSignal,
    ) => Promise<void>;
    onBack: () => void;
    onRemove: () => void;
    onReload: () => void;
  },
): React.JSX.Element {
  const [selection, setSelection] = useState("");
  const choice = available.find((item) => item.key === selection) ??
    available[0];
  const capable = choice?.extension.views.some((view) =>
    view.id === choice.view && view.discovery
  );
  const [relation, setRelation] = useState("author");
  const [state, setState] = useState("open");
  const [scope, setScope] = useState("all");
  const [panel, setPanel] = useState<"filters" | "link" | null>(null);
  const [input, setInput] = useState("");
  const [loaded, setLoaded] = useState<{ key: string; page: PullPage }>();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string>();
  const [opening, setOpening] = useState<string>();
  const [openError, setOpenError] = useState<string>();
  const [revision, setRevision] = useState(0);
  const pending = useRef<AbortController | undefined>(undefined);
  const association = useRef<AbortController | undefined>(undefined);
  const key = JSON.stringify([
    context,
    choice?.key,
    choice?.extension.identity,
    relation,
    state,
    scope,
  ]);
  const snapshot = loaded?.key === key ? loaded.page : undefined;
  async function load(page: number, previous?: PullPage) {
    if (!choice || !capable) return;
    pending.current?.abort();
    const controller = new AbortController();
    pending.current = controller;
    setLoading(true);
    setError(undefined);
    try {
      const query = resourceQuery(
        choice.extension.identity,
        choice.remote.name,
        choice.view,
        "",
        page,
      );
      query.set("discovery", "true");
      query.set("relation", relation);
      query.set("state", state);
      query.set("currentRepository", String(scope === "current"));
      if (previous) query.set("account", previous.account);
      const response = await extensionRequest(
        context,
        query,
        controller.signal,
      );
      if (controller.signal.aborted) return;
      if (response.type !== "pulls") {
        throw new Error("Update this Machine to browse pull requests.");
      }
      if (previous && response.account !== previous.account) {
        throw new Error("GitHub account changed. Refresh the list.");
      }
      for (const item of response.items) {
        const [owner, repository] = item.repository.split("/");
        if (
          pullNumber(item.url, {
            host: choice.remote.host,
            owner: owner!,
            repository: repository!,
          }) !== item.number
        ) throw new Error("Unexpected PR repository. Refresh the list.");
      }
      const items = previous
        ? [
          ...new Map(
            [...previous.items, ...response.items].map((
              item,
            ) => [item.url, item]),
          ).values(),
        ]
        : response.items;
      setLoaded({ key, page: { ...response, items } });
    } catch (error) {
      if (!controller.signal.aborted) {
        setError(
          error instanceof Error
            ? error.message
            : "Could not load pull requests.",
        );
      }
    } finally {
      if (!controller.signal.aborted) setLoading(false);
    }
  }
  const loadRef = useRef(load);
  loadRef.current = load;
  useEffect(() => {
    setLoaded(undefined);
    setLoading(false);
    setError(undefined);
    setOpenError(undefined);
    setOpening(undefined);
    association.current?.abort();
    void loadRef.current(1);
    return () => {
      pending.current?.abort();
      association.current?.abort();
    };
  }, [key, revision]);
  async function select(target: Choice, number: string, label: string) {
    association.current?.abort();
    const controller = new AbortController();
    association.current = controller;
    setOpening(label);
    setOpenError(undefined);
    try {
      await onSelect(target, number, controller.signal);
    } catch (error) {
      if (!controller.signal.aborted) {
        setOpenError(
          error instanceof Error ? error.message : "Could not open PR.",
        );
      }
    } finally {
      if (!controller.signal.aborted) setOpening(undefined);
    }
  }
  function pick(item: PullSummary) {
    if (!choice) return;
    const [owner, repository] = item.repository.split("/");
    void select(
      {
        ...choice,
        remote: { ...choice.remote, owner: owner!, repository: repository! },
      },
      item.number,
      item.url,
    );
  }
  function openLink() {
    if (!choice) return;
    try {
      let target = choice;
      if (input.trim().startsWith("https://") && capable) {
        const url = new URL(input.trim());
        const parts = url.pathname.split("/");
        target = {
          ...choice,
          remote: {
            ...choice.remote,
            owner: parts[1] ?? "",
            repository: parts[2] ?? "",
          },
        };
      }
      const number = pullNumber(input, target.remote);
      void select(target, number, "link");
    } catch {
      setOpenError(
        "Enter a PR number for the selected repository, or a full PR URL on this GitHub host.",
      );
    }
  }
  const label = relation === "author"
    ? "My pull requests"
    : relation === "review"
    ? "Review requested"
    : relation === "assigned"
    ? "Assigned to me"
    : "Pull requests";
  return (
    <Stack sx={{ flex: 1, minHeight: 0 }}>
      <Stack
        direction="row"
        justifyContent="space-between"
        alignItems="center"
        sx={{ px: 2, py: 1.5 }}
      >
        <Box>
          <Typography fontWeight={600}>{label}</Typography>
          <Typography variant="caption" color="text.secondary">
            {snapshot ? `@${snapshot.account} · ` : ""}
            {state === "all"
              ? "All states"
              : state[0]!.toUpperCase() + state.slice(1)} · {scope === "all"
              ? "All repositories"
              : `${choice?.remote.owner}/${choice?.remote.repository}`}
          </Typography>
        </Box>
        {loading && snapshot && (
          <Typography role="status" variant="caption">Updating…</Typography>
        )}
      </Stack>
      {openError && (
        <Alert
          severity="warning"
          onClose={() => setOpenError(undefined)}
        >
          {openError}
        </Alert>
      )}
      {error && (
        <Alert
          severity="warning"
          action={
            <Stack>
              <Button onClick={() => void load(1)}>Retry</Button>
              <Button onClick={onReload}>Reload connection</Button>
            </Stack>
          }
        >
          {error}
        </Alert>
      )}
      <Box
        sx={{ ...mobileNativeYScrollSx, flex: 1, minHeight: 0 }}
        aria-busy={loading}
      >
        {!capable
          ? (
            <Alert severity="info" sx={{ m: 2 }}>
              Update this Machine to browse your PRs. You can still open a PR by
              link below.
            </Alert>
          )
          : !snapshot && !error
          ? <PullSkeleton />
          : snapshot && (
            <>
              {!snapshot.items.length && (
                <Stack spacing={1} sx={{ p: 3, pt: 5 }}>
                  <Typography variant="h6">
                    No matching pull requests
                  </Typography>
                  <Typography color="text.secondary">
                    Try another status or relationship, or open a PR by link.
                  </Typography>
                  <Button
                    onClick={() =>
                      setPanel("filters")}
                  >
                    Change filters
                  </Button>
                </Stack>
              )}
              <List disablePadding aria-label="Pull requests">
                {snapshot.items.map((item) => (
                  <ListItemButton
                    key={item.url}
                    disabled={opening !== undefined}
                    aria-label={`Review ${item.repository} #${item.number}`}
                    onClick={() => pick(item)}
                    sx={{
                      px: 2,
                      py: 1.5,
                      borderBottom: 1,
                      borderColor: "divider",
                      alignItems: "center",
                      minHeight: 88,
                    }}
                  >
                    <Box sx={{ flex: 1, minWidth: 0 }}>
                      <Typography variant="caption" color="text.secondary">
                        {item.repository} #{item.number}
                      </Typography>
                      <Typography
                        sx={{
                          fontWeight: 600,
                          overflowWrap: "anywhere",
                          my: 0.5,
                        }}
                      >
                        {item.title}
                      </Typography>
                      <Typography variant="caption" color="text.secondary">
                        {opening === item.url
                          ? "Opening PR…"
                          : `${
                            item.draft ? "Draft" : item.state
                          } · @${item.author} · ${
                            new Date(item.updatedAt).toLocaleDateString()
                          }`}
                      </Typography>
                    </Box>
                    <ChevronRight fontSize="small" sx={{ ml: 1 }} />
                  </ListItemButton>
                ))}
              </List>
              {(snapshot.incomplete || snapshot.total > 1000) && (
                <Alert severity="info" sx={{ m: 2 }}>
                  GitHub returned limited search results. Narrow the filters to
                  find more PRs.
                </Alert>
              )}
              {snapshot.nextPage && (
                <Button
                  fullWidth
                  sx={{ minHeight: 48 }}
                  disabled={loading || !!opening}
                  onClick={() => void load(snapshot.nextPage!, snapshot)}
                >
                  {loading ? "Loading more…" : "Load more pull requests"}
                </Button>
              )}
              {!!snapshot.items.length && (
                <Typography
                  variant="caption"
                  color="text.secondary"
                  sx={{ display: "block", p: 2 }}
                >
                  {snapshot.items.length} of {snapshot.total}{" "}
                  results · Updated most recently
                </Typography>
              )}
            </>
          )}
      </Box>
      {panel && (
        <Stack
          spacing={1.5}
          sx={{
            p: 2,
            borderTop: 1,
            borderColor: "divider",
            maxHeight: "50%",
            ...mobileNativeYScrollSx,
          }}
        >
          {panel === "filters"
            ? (
              <>
                <Stack direction="row" spacing={1}>
                  <TextField
                    select
                    size="small"
                    label="Relationship"
                    fullWidth
                    value={relation}
                    onChange={(e) => setRelation(e.target.value)}
                  >
                    <MenuItem value="author">Created by me</MenuItem>
                    <MenuItem value="review">Review requested</MenuItem>
                    <MenuItem value="assigned">Assigned to me</MenuItem>
                    <MenuItem value="all">
                      {scope === "all" ? "Involving me" : "Everyone"}
                    </MenuItem>
                  </TextField>
                  <TextField
                    select
                    size="small"
                    label="Status"
                    fullWidth
                    value={state}
                    onChange={(e) => setState(e.target.value)}
                  >
                    <MenuItem value="open">Open</MenuItem>
                    <MenuItem value="merged">Merged</MenuItem>
                    <MenuItem value="closed">Closed</MenuItem>
                    <MenuItem value="all">All states</MenuItem>
                  </TextField>
                </Stack>
                <TextField
                  select
                  size="small"
                  label="Scope"
                  value={scope}
                  onChange={(e) => setScope(e.target.value)}
                >
                  <MenuItem value="all">All repositories</MenuItem>
                  <MenuItem value="current">Selected repository</MenuItem>
                </TextField>
                {available.length > 1 && (
                  <TextField
                    select
                    size="small"
                    label="Repository / connection"
                    value={choice?.key ?? ""}
                    onChange={(e) => setSelection(e.target.value)}
                  >
                    {available.map((c) => (
                      <MenuItem key={c.key} value={c.key}>
                        {c.remote.owner}/{c.remote.repository} ·{" "}
                        {c.extension.label}
                      </MenuItem>
                    ))}
                  </TextField>
                )}
                <Button onClick={() => setPanel(null)}>Done</Button>
              </>
            )
            : (
              <>
                <TextField
                  size="small"
                  label="PR URL or number"
                  value={input}
                  disabled={!!opening}
                  onChange={(e) => setInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && !opening) openLink();
                  }}
                  helperText={`Numbers use ${choice?.remote.owner}/${choice?.remote.repository}`}
                />
                <Button
                  variant="contained"
                  disableElevation
                  disabled={!choice || !input.trim() || !!opening}
                  onClick={openLink}
                >
                  {opening === "link" ? "Opening PR…" : "Open pull request"}
                </Button>
                {bound && (
                  <Button color="error" onClick={onRemove}>
                    Remove association
                  </Button>
                )}
              </>
            )}
        </Stack>
      )}
      <Stack
        direction="row"
        alignItems="center"
        spacing={0.5}
        sx={remoteBottomSx}
      >
        <Button
          startIcon={<ArrowBack />}
          aria-label={bound ? "Back to PR" : "Local worktree review"}
          onClick={onBack}
          sx={{ minHeight: 44 }}
        >
          Back
        </Button>
        <Box sx={{ flex: 1 }} />
        <Button
          startIcon={<FilterList />}
          aria-pressed={panel === "filters"}
          onClick={() => setPanel(panel === "filters" ? null : "filters")}
          sx={{ minHeight: 44 }}
        >
          Filters
        </Button>
        <IconButton
          aria-label="Open PR by link"
          onClick={() => setPanel(panel === "link" ? null : "link")}
        >
          <Link />
        </IconButton>
        <IconButton
          aria-label="Refresh pull requests"
          disabled={loading || !!opening}
          onClick={() => snapshot ? void load(1) : setRevision((n) => n + 1)}
        >
          <Refresh />
        </IconButton>
        <WorkspaceExtensionsButton context={context} />
      </Stack>
    </Stack>
  );
}
