import { desktopSize } from "../surface/desktopSize";
import {
  ArrowBack,
  ChevronLeft,
  ChevronRight,
  OpenInNew,
  Refresh,
  SettingsOutlined,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Divider,
  IconButton,
  List,
  ListItemButton,
  ListItemText,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useState } from "react";
import { Markdown } from "../Markdown";
import { useSurfaceProfile } from "../surface/SurfaceProfile";
import {
  extensionRequest,
  type ExtensionResponse,
  type RepositoryRemote,
  type Resource,
  resourceQuery,
  type WorkspaceExtension,
} from "./api";
import { ExtensionManager } from "./ExtensionManager";
import { HintTooltip } from "../HintTooltip";

type Result = Exclude<ExtensionResponse, { type: "unavailable" }>;
function useRead(context: string, query: string | null, refresh: number) {
  const key = JSON.stringify([context, query, refresh]);
  const [state, setState] = useState<
    { key: string; result?: Result; error?: string }
  >();
  useEffect(() => {
    const controller = new AbortController();
    void extensionRequest(
      context,
      query === null ? null : new URLSearchParams(query),
      controller.signal,
    ).then(
      (result) => {
        if (!controller.signal.aborted) setState({ key, result });
      },
      (error: unknown) => {
        if (!controller.signal.aborted) {
          setState({
            key,
            error: error instanceof Error
              ? error.message
              : "Extension unavailable",
          });
        }
      },
    );
    return () => controller.abort();
  }, [context, key, query]);
  // Never paint a response belonging to a previous selection, even for one frame.
  return state?.key === key ? state : { key };
}

function Loading(): React.JSX.Element {
  return (
    <Stack role="status" alignItems="center" sx={{ p: 4 }}>
      <CircularProgress size={desktopSize(24)} />
      <Typography sx={{ mt: 1 }} color="text.secondary">
        Loading resources…
      </Typography>
    </Stack>
  );
}
function Empty({ children }: { children: React.ReactNode }): React.JSX.Element {
  return (
    <Typography color="text.secondary" sx={{ p: 3, textAlign: "center" }}>
      {children}
    </Typography>
  );
}

function ResourceDetail(
  { context, extension, remote, view, item, refresh, onBack }: {
    context: string;
    extension: WorkspaceExtension;
    remote: string;
    view: string;
    item: string;
    refresh: number;
    onBack: () => void;
  },
): React.JSX.Element {
  const read = useRead(
    context,
    resourceQuery(extension.identity, remote, view, "", 1, item).toString(),
    refresh,
  );
  const detail = read.result?.type === "detail" ? read.result.item : null;
  return (
    <Stack sx={{ minWidth: 0, flex: 1, overflow: "auto", p: 2 }} spacing={2}>
      <Button
        startIcon={<ArrowBack />}
        onClick={onBack}
        sx={{ alignSelf: "flex-start" }}
      >
        Back to list
      </Button>
      {read.error
        ? <Alert severity="warning">{read.error}</Alert>
        : !detail
        ? <Loading />
        : (
          <>
            <Stack direction="row" spacing={1} alignItems="flex-start">
              <Typography
                component="h2"
                variant="h6"
                sx={{ flex: 1, overflowWrap: "anywhere" }}
              >
                {detail.title}
              </Typography>
              {detail.url && (
                <HintTooltip title="Open original resource">
                  <IconButton
                    component="a"
                    href={detail.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    aria-label="Open original resource"
                  >
                    <OpenInNew />
                  </IconButton>
                </HintTooltip>
              )}
            </Stack>
            <Stack direction="row" spacing={1} alignItems="center">
              <Typography color="text.secondary">#{detail.id}</Typography>
              {detail.state && <Chip size="small" label={detail.state} />}
            </Stack>
            <Box
              component="dl"
              sx={{
                display: "grid",
                gridTemplateColumns: "max-content minmax(0,1fr)",
                gap: 1,
                m: 0,
                fontSize: desktopSize(13),
              }}
            >
              {detail.metadata.map((row, index) => (
                <Box key={index} sx={{ display: "contents" }}>
                  <Typography
                    component="dt"
                    variant="body2"
                    color="text.secondary"
                  >
                    {row.label}
                  </Typography>
                  <Typography
                    component="dd"
                    variant="body2"
                    sx={{ m: 0, overflowWrap: "anywhere" }}
                  >
                    {row.value}
                  </Typography>
                </Box>
              ))}
            </Box>
            <Divider />
            {detail.bodyTruncated && (
              <Alert severity="info">
                This description is long. Open the original resource to read it
                in full.
              </Alert>
            )}
            {detail.body
              ? <Markdown text={detail.body} touchWrap />
              : <Typography color="text.secondary">No description.</Typography>}
          </>
        )}
    </Stack>
  );
}

function Resources({ context, extension, remotes, onBack }: {
  context: string;
  extension: WorkspaceExtension;
  remotes: RepositoryRemote[];
  onBack: () => void;
}): React.JSX.Element {
  const desktop = useSurfaceProfile().kind === "desktop";
  const [remote, setRemote] = useState(remotes[0]?.name ?? "");
  const [viewId, setViewId] = useState(extension.views[0]?.id ?? "");
  const view = extension.views.find((candidate) => candidate.id === viewId)!;
  const [filter, setFilter] = useState(view?.filters[0]?.value ?? "");
  const [page, setPage] = useState(1);
  const [search, setSearch] = useState("");
  const [item, setItem] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  function reset() {
    setPage(1);
    setItem(null);
    setSearch("");
  }
  return (
    <Stack sx={{ flex: 1, minHeight: 0 }}>
      <Stack direction="row" alignItems="center" spacing={1} sx={{ p: 1 }}>
        <IconButton onClick={onBack} aria-label="All extensions">
          <ArrowBack />
        </IconButton>
        <Typography fontWeight={650} sx={{ flex: 1 }}>
          {extension.label}
        </Typography>
        <IconButton
          onClick={() => setRefresh((v) => v + 1)}
          aria-label="Refresh resources"
        >
          <Refresh />
        </IconButton>
      </Stack>
      {!extension.available
        ? (
          <Alert severity="warning" sx={{ m: 2 }}>
            A required Plugin is unavailable. Check the installed extensions and
            dependencies on this Machine.
          </Alert>
        )
        : !remotes.length
        ? <Empty>This workspace has no supported repository remote.</Empty>
        : !view
        ? <Empty>No resource views are available.</Empty>
        : (
          <>
            <Stack
              direction={desktop ? "row" : "column"}
              spacing={1}
              sx={{ px: 2, pb: 1.5 }}
            >
              <TextField
                select
                size="small"
                label="Repository"
                value={remote}
                onChange={(e) => {
                  setRemote(e.target.value);
                  reset();
                }}
                sx={{ flex: 1, minWidth: 0 }}
              >
                {remotes.map((r) => (
                  <MenuItem key={r.name} value={r.name}>
                    {r.name} · {r.owner}/{r.repository} · {r.host}
                  </MenuItem>
                ))}
              </TextField>
              <Stack direction="row" spacing={1} sx={{ flex: 1 }}>
                <TextField
                  select
                  size="small"
                  label="View"
                  value={viewId}
                  onChange={(e) => {
                    const next = extension.views.find((v) =>
                      v.id === e.target.value
                    )!;
                    setViewId(next.id);
                    setFilter(next.filters[0]?.value ?? "");
                    reset();
                  }}
                  sx={{ flex: 1 }}
                >
                  {extension.views.map((v) => (
                    <MenuItem key={v.id} value={v.id}>{v.label}</MenuItem>
                  ))}
                </TextField>
                {view.filters.length > 0 && (
                  <TextField
                    select
                    size="small"
                    label="Status"
                    value={filter}
                    onChange={(e) => {
                      setFilter(e.target.value);
                      reset();
                    }}
                    sx={{ minWidth: 110 }}
                  >
                    {view.filters.map((f) => (
                      <MenuItem key={f.value} value={f.value}>
                        {f.label}
                      </MenuItem>
                    ))}
                  </TextField>
                )}
              </Stack>
            </Stack>
            <Divider />
            <Stack
              direction="row"
              sx={{ flex: 1, minHeight: 0, overflow: "hidden" }}
            >
              {(desktop || !item) && (
                <ResourceList
                  context={context}
                  extension={extension}
                  remote={remote}
                  view={view.id}
                  filter={filter}
                  page={page}
                  search={search}
                  refresh={refresh}
                  selected={item}
                  onSelect={setItem}
                  onSearch={setSearch}
                  onPage={(p) => {
                    setPage(p);
                    setItem(null);
                  }}
                  split={desktop && item !== null}
                />
              )}
              {item && (
                <ResourceDetail
                  key={JSON.stringify([remote, viewId, item])}
                  context={context}
                  extension={extension}
                  remote={remote}
                  view={viewId}
                  item={item}
                  refresh={refresh}
                  onBack={() => setItem(null)}
                />
              )}
            </Stack>
          </>
        )}
    </Stack>
  );
}

function ResourceList(
  {
    context,
    extension,
    remote,
    view,
    filter,
    page,
    search,
    refresh,
    selected,
    onSelect,
    onSearch,
    onPage,
    split,
  }: {
    context: string;
    extension: WorkspaceExtension;
    remote: string;
    view: string;
    filter: string;
    page: number;
    search: string;
    refresh: number;
    selected: string | null;
    onSelect: (id: string) => void;
    onSearch: (value: string) => void;
    onPage: (page: number) => void;
    split: boolean;
  },
): React.JSX.Element {
  const read = useRead(
    context,
    resourceQuery(extension.identity, remote, view, filter, page).toString(),
    refresh,
  );
  const result = read.result?.type === "page" ? read.result : null;
  const items =
    result?.items.filter((r: Resource) =>
      `${r.id} ${r.title}`.toLowerCase().includes(search.toLowerCase())
    ) ?? [];
  return (
    <Stack
      sx={{
        flex: split ? "0 0 38%" : 1,
        minWidth: 0,
        borderRight: split ? 1 : 0,
        borderColor: "divider",
      }}
    >
      <TextField
        size="small"
        label="Filter this page"
        value={search}
        onChange={(e) => onSearch(e.target.value)}
        sx={{ m: 1.5 }}
      />
      <Box sx={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        {read.error
          ? <Alert severity="warning" sx={{ m: 1.5 }}>{read.error}</Alert>
          : !result
          ? <Loading />
          : !items.length
          ? (
            <Empty>
              {search
                ? "No matching resources on this page."
                : "No resources on this page."}
            </Empty>
          )
          : (
            <List disablePadding>
              {items.map((r) => (
                <ListItemButton
                  key={r.id}
                  selected={selected === r.id}
                  onClick={() => onSelect(r.id)}
                  alignItems="flex-start"
                  sx={{ minHeight: 60, py: 1.25 }}
                >
                  <ListItemText
                    primary={r.title}
                    secondary={`#${r.id}${r.state ? ` · ${r.state}` : ""}${
                      r.updatedAt
                        ? ` · ${new Date(r.updatedAt).toLocaleDateString()}`
                        : ""
                    }`}
                    slotProps={{
                      primary: {
                        sx: { overflowWrap: "anywhere", fontSize: desktopSize(14) },
                      },
                      secondary: { sx: { mt: 0.5 } },
                    }}
                  />
                </ListItemButton>
              ))}
            </List>
          )}
      </Box>
      <Stack
        direction="row"
        alignItems="center"
        justifyContent="space-between"
        sx={{ p: 1, borderTop: 1, borderColor: "divider" }}
      >
        <IconButton
          aria-label="Previous page"
          disabled={page === 1}
          onClick={() => onPage(page - 1)}
        >
          <ChevronLeft />
        </IconButton>
        <Typography variant="caption" color="text.secondary">
          Page {page}
        </Typography>
        <IconButton
          aria-label="Next page"
          disabled={!result?.nextPage}
          onClick={() => {
            if (result?.nextPage) onPage(result.nextPage);
          }}
        >
          <ChevronRight />
        </IconButton>
      </Stack>
    </Stack>
  );
}

export default function WorkspaceExtensions(
  { context, machineId }: { context: string; machineId?: string | undefined },
): React.JSX.Element {
  const [refresh, setRefresh] = useState(0);
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [manage, setManage] = useState(false);
  const read = useRead(context, null, refresh);
  const inventory = read.result?.type === "inventory" ? read.result : null;
  const extension = inventory?.extensions.find((e) =>
    e.identity.pluginId === selected
  );
  if (manage) {
    return (
      <Stack sx={{ p: 2, overflow: "auto" }} spacing={2}>
        <Button
          startIcon={<ArrowBack />}
          onClick={() => {
            setManage(false);
            setRefresh((v) => v + 1);
          }}
          sx={{ alignSelf: "flex-start" }}
        >
          Back to extensions
        </Button>
        <ExtensionManager initialMachineId={machineId} />
      </Stack>
    );
  }
  if (extension && inventory) {
    return (
      <Resources
        key={JSON.stringify(extension.identity)}
        context={context}
        extension={extension}
        remotes={inventory.remotes}
        onBack={() => setSelected(null)}
      />
    );
  }
  return (
    <Stack spacing={2} sx={{ p: 2, flex: 1, minHeight: 0, overflow: "auto" }}>
      <Stack direction="row" spacing={1} alignItems="center">
        <TextField
          label="Find an extension"
          size="small"
          value={search}
          onChange={(e) =>
            setSearch(e.target.value)}
          sx={{ flex: 1 }}
        />
        <IconButton
          aria-label="Refresh extensions"
          onClick={() =>
            setRefresh((v) => v + 1)}
        >
          <Refresh />
        </IconButton>
        <HintTooltip title="Manage extensions">
          <IconButton
            aria-label="Manage extensions"
            onClick={() => setManage(true)}
          >
            <SettingsOutlined />
          </IconButton>
        </HintTooltip>
      </Stack>
      {read.error
        ? <Alert severity="warning">{read.error}</Alert>
        : !inventory
        ? <Loading />
        : !inventory.extensions.length
        ? (
          <>
            <Empty>
              No extensions are installed on this workspace’s Machine.
            </Empty>
            <Button onClick={() => setManage(true)}>Browse extensions</Button>
          </>
        )
        : (
          <List disablePadding>
            {inventory.extensions.filter((e) =>
              `${e.label} ${e.description}`.toLowerCase().includes(
                search.toLowerCase(),
              )
            ).map((e) => (
              <ListItemButton
                key={e.identity.pluginId}
                onClick={() => setSelected(e.identity.pluginId)}
                sx={{ minHeight: 72, borderBottom: 1, borderColor: "divider" }}
              >
                <ListItemText
                  primary={e.label}
                  secondary={e.available
                    ? e.description
                    : "Required Plugin unavailable"}
                />
                <ChevronRight color="action" />
              </ListItemButton>
            ))}
          </List>
        )}
    </Stack>
  );
}
