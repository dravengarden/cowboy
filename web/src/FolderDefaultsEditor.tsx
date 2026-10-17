import { useEffect, useRef, useState } from "react";
import {
  Alert,
  Box,
  Button,
  Divider,
  IconButton,
  InputAdornment,
  List,
  ListItemButton,
  ListItemText,
  MenuItem,
  MenuList,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import {
  ArrowBack,
  Check,
  ChevronRight,
  ExpandMore,
  Search,
} from "@mui/icons-material";
import {
  FOLDER_DEFAULT_FIELDS,
  type FolderDefaultKey,
  resolveFolderDefaults,
} from "./folderDefaults";
import { sessionFolderById, type SessionFoldersValue } from "./sessionFolders";
import type { MachineSummary } from "./protocol";
import { latestProviderEntries, useProviderCatalog } from "./providerCatalog";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { useDialogFocus } from "./useDialogInputFocus";
import { directoryProjectRows, directoryProjects } from "./directoryProjects";
import { ReviewDirectoryPicker } from "./ReviewDirectoryPicker";
import { reviewDirectoryLabel } from "./reviewDirectoryDefault";
import { useStoreSelector } from "./store";

const automatic: Record<FolderDefaultKey, string> = {
  project: "Choose when creating",
  machine: "Choose with project",
  runtime_machine: "Global placement default",
  provider: "Claude",
  preset: "Agent recommended preset",
  review_directory: "Automatic · session worktree, then project",
};

export function FolderDefaultsEditor(
  {
    id,
    tree,
    machines,
    catalog,
    error,
    onSetDefault,
    field,
    onFieldChange,
    draft = false,
    fill = false,
  }: {
    id: string;
    tree: SessionFoldersValue;
    machines: readonly MachineSummary[];
    catalog: ReturnType<typeof useProviderCatalog>["catalog"];
    error?: string | null;
    onSetDefault: (
      id: string,
      key: FolderDefaultKey,
      value: string | null | undefined,
    ) => void;
    field: FolderDefaultKey | null;
    onFieldChange: (field: FolderDefaultKey | null) => void;
    draft?: boolean;
    fill?: boolean;
  },
): React.JSX.Element {
  const folder = sessionFolderById(tree, id);
  const sessions = useStoreSelector((state) => state.sessions);
  const entries = latestProviderEntries(catalog?.providers ?? []);
  const desktop = useSurfaceProfile().kind === "desktop";
  const [search, setSearch] = useState("");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const content = useRef<HTMLDivElement>(null);
  const previousField = useRef(field);
  useDialogFocus(
    () =>
      fill
        ? content.current?.querySelector<HTMLElement>("[data-default-field]")
        : null,
    desktop && fill,
  );
  useEffect(() => {
    const previous = previousField.current;
    previousField.current = field;
    if (field) setSearch("");
    const frame = requestAnimationFrame(() => {
      if (!field) {
        if (previous) {
          content.current?.querySelector<HTMLElement>(
            `[data-default-field="${previous}"]`,
          )?.focus();
        }
        return;
      }
      content.current?.querySelector<HTMLElement>(".Mui-selected")
        ?.scrollIntoView({ block: "nearest" });
      if (desktop) {
        content.current?.querySelector<HTMLInputElement>(
          'input[aria-label="Search defaults"]',
        )?.focus();
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [field, desktop]);
  const withoutOverride = { ...folder?.defaults };
  if (field) delete withoutOverride[field];
  const inherited = resolveFolderDefaults({
    ...tree,
    folders: tree.folders.map((entry) =>
      entry.id === id ? { ...entry, defaults: withoutOverride } : entry
    ),
  }, id);
  const effective = resolveFolderDefaults(tree, id);
  const projects = directoryProjects(machines, effective.values.machine);
  const provider = effective.values.provider ?? "claude-code";
  const choices: Record<
    FolderDefaultKey,
    { value: string; label: string; detail?: string }[]
  > = {
    review_directory: [],
    project: projects,
    machine: machines.map((machine) => ({
      value: machine.id,
      label: machine.display_name,
      detail: [
        machine.connected ? "Online" : "Offline",
        effective.values.project &&
          !machine.workspaces.some((p) =>
            p.id === effective.values.project ||
            p.display_name === effective.values.project
          )
          ? "Project unavailable"
          : "",
      ].filter(Boolean).join(" · "),
    })),
    runtime_machine: machines.map((machine) => ({
      value: machine.id,
      label: machine.display_name,
      detail: machine.connected ? "Online" : "Offline",
    })),
    provider: entries.map((entry) => ({
      value: entry.provider_id,
      label: entry.manifest.display.name,
    })),
    preset: entries.filter((entry) => entry.provider_id === provider).flatMap((
      entry,
    ) =>
      entry.manifest.configuration.presets.map((preset) => ({
        value: `${entry.provider_id}/${preset.id}`,
        label: preset.name,
        detail: preset.detail,
      }))
    ),
  };
  const label = (key: FolderDefaultKey, value: string | null | undefined) =>
    key === "review_directory"
      ? reviewDirectoryLabel(value, machines, sessions)
      : value
      ? choices[key].find((option) => option.value === value)?.label ?? value
      : automatic[key];
  const source = (key: FolderDefaultKey) => {
    const owner = effective.sources[key];
    if (
      owner === id && key === "project" && folder?.project &&
      folder.defaults?.project === undefined
    ) return "Auto-file project binding";
    return owner === id
      ? "This directory"
      : owner
      ? `Inherited from ${
        sessionFolderById(tree, owner)?.name ?? "parent directory"
      }`
      : "Global default";
  };
  const openField = (key: FolderDefaultKey) => {
    setSearch("");
    const opened = new Set<string>();
    let cursor = effective.values.project;
    while (cursor && !opened.has(cursor)) {
      opened.add(cursor);
      cursor = projects.find((project) => project.value === cursor)?.parent;
    }
    // Show the actual hierarchy immediately, including registered parent projects.
    for (const project of projects) {
      if (!project.parent) opened.add(project.value);
    }
    setExpanded(opened);
    onFieldChange(key);
  };
  const back = () => {
    onFieldChange(null);
  };
  const save = (value: string | null | undefined) => {
    if (!field || !folder) return;
    onSetDefault(id, field, value);
    back();
  };
  const presetMismatch = effective.values.preset &&
    !effective.values.preset.startsWith(`${provider}/`);
  const missingProject = effective.values.project &&
    !projects.some((p) =>
      p.value === effective.values.project ||
      p.detail.startsWith(`${effective.values.project} ·`)
    );
  const rows = directoryProjectRows(projects, expanded, search);
  const own = field ? folder?.defaults?.[field] : undefined;
  return (
    <Stack
      ref={content}
      spacing={1.5}
      sx={{
        minHeight: 0,
        flex: 1,
        height: fill
          ? (desktop ? "min(65dvh, 540px)" : "100%")
          : field
          ? "min(54dvh, 440px)"
          : undefined,
      }}
    >
      {!folder
        ? <Alert severity="error">This directory was removed.</Alert>
        : field === "review_directory"
        ? (
          <>
            <Button
              startIcon={<ArrowBack />}
              onClick={back}
              sx={{ alignSelf: "flex-start" }}
            >
              {draft ? "Back to folder" : `${folder.name} defaults`}
            </Button>
            <Typography variant="body2" color="text.secondary">
              Choose the code directory Zed opens. Sessions inherit this setting
              unless they override it.
            </Typography>
            <ReviewDirectoryPicker
              value={own}
              inherited={inherited.values.review_directory}
              machines={machines}
              sessions={sessions}
              onChange={save}
            />
          </>
        )
        : field
        ? (
          <>
            <Button
              startIcon={<ArrowBack />}
              onClick={back}
              sx={{ alignSelf: "flex-start" }}
            >
              {draft ? "Back to folder" : `${folder.name} defaults`}
            </Button>
            <Stack spacing={0.5}>
              <Button
                data-default-mode="inherit"
                variant={own === undefined ? "contained" : "text"}
                onClick={() => save(undefined)}
                sx={{ justifyContent: "flex-start" }}
              >
                {inherited.sources[field] === id
                  ? "Use auto-file binding"
                  : "Inherit"} · {label(field, inherited.values[field])}
              </Button>
              <Button
                data-default-mode="clear"
                variant={own === null ? "contained" : "text"}
                onClick={() => save(null)}
                sx={{ justifyContent: "flex-start" }}
              >
                {field === "project"
                  ? "No project — choose when creating"
                  : "Use global default"}
              </Button>
            </Stack>
            <TextField
              size="small"
              fullWidth
              placeholder={`Search ${
                FOLDER_DEFAULT_FIELDS[field].toLowerCase()
              }`}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              slotProps={{
                input: {
                  startAdornment: (
                    <InputAdornment position="start">
                      <Search fontSize="small" />
                    </InputAdornment>
                  ),
                },
                htmlInput: { "aria-label": "Search defaults" },
              }}
            />
            <Typography variant="caption" color="text.secondary">
              Choose a value for this directory
            </Typography>
            <Box
              sx={{
                overflowY: "auto",
                minHeight: 0,
                flex: 1,
                overscrollBehavior: "contain",
              }}
            >
              <MenuList
                aria-label={`${FOLDER_DEFAULT_FIELDS[field]} choices`}
                disablePadding
              >
                {field === "project"
                  ? rows.map(({ project, depth, branch }) => (
                    <MenuItem
                      key={project.value}
                      data-default-value={project.value}
                      selected={effective.values.project === project.value}
                      onClick={() => save(project.value)}
                      sx={{
                        ml: depth * 2.5,
                        pl: 0.5,
                        py: 0.5,
                        gap: 0.5,
                        borderLeft: depth ? "1px solid" : undefined,
                        borderColor: "divider",
                      }}
                    >
                      {branch
                        ? (
                          <IconButton
                            size="small"
                            sx={{
                              width: desktop ? 32 : 44,
                              height: desktop ? 32 : 44,
                              flexShrink: 0,
                            }}
                            aria-label={`${
                              expanded.has(project.value)
                                ? "Collapse"
                                : "Expand"
                            } ${project.label}`}
                            aria-expanded={expanded.has(project.value)}
                            onClick={(event) => {
                              event.stopPropagation();
                              setExpanded((current) => {
                                const next = new Set(current);
                                if (next.has(project.value)) {
                                  next.delete(project.value);
                                } else next.add(project.value);
                                return next;
                              });
                            }}
                          >
                            {expanded.has(project.value)
                              ? <ExpandMore />
                              : <ChevronRight />}
                          </IconButton>
                        )
                        : (
                          <Box
                            sx={{ width: desktop ? 32 : 44, flexShrink: 0 }}
                          />
                        )}
                      <ListItemText
                        primary={project.label}
                        secondary={project.detail}
                        slotProps={{
                          primary: {
                            sx: {
                              overflowWrap: "anywhere",
                              whiteSpace: "normal",
                            },
                          },
                          secondary: {
                            sx: {
                              overflowWrap: "anywhere",
                              whiteSpace: "normal",
                            },
                          },
                        }}
                      />
                      {effective.values.project === project.value && (
                        <Check fontSize="small" />
                      )}
                    </MenuItem>
                  ))
                  : choices[field].filter((option) =>
                    `${option.label} ${option.detail ?? ""}`
                      .toLocaleLowerCase().includes(
                        search.trim().toLocaleLowerCase(),
                      )
                  ).map((option) => (
                    <MenuItem
                      key={option.value}
                      data-default-value={option.value}
                      selected={effective.values[field] === option.value}
                      onClick={() => save(option.value)}
                      sx={{ py: 1 }}
                    >
                      <ListItemText
                        primary={option.label}
                        secondary={option.detail}
                        slotProps={{
                          primary: { sx: { whiteSpace: "normal" } },
                          secondary: { sx: { whiteSpace: "normal" } },
                        }}
                      />
                      {effective.values[field] === option.value && (
                        <Check fontSize="small" />
                      )}
                    </MenuItem>
                  ))}
              </MenuList>
              {(field === "project"
                ? rows.length === 0
                : !choices[field].some((option) =>
                  `${option.label} ${option.detail ?? ""}`
                    .toLocaleLowerCase().includes(
                      search.trim().toLocaleLowerCase(),
                    )
                )) && (
                <Typography sx={{ p: 2 }} color="text.secondary">
                  No matching choices. Check the Machine connection or choose
                  another default.
                </Typography>
              )}
            </Box>
          </>
        )
        : (
          <>
            <Typography variant="body2" color="text.secondary">
              {draft
                ? "New sessions and subfolders inherit these settings."
                : "Subdirectories inherit these defaults. Agent defaults apply when creating sessions; Zed defaults also follow the directory in existing sessions."}
            </Typography>
            {error && <Alert severity="error">{error}</Alert>}
            {presetMismatch && !draft && (
              <Alert
                severity="warning"
                action={
                  <Button onClick={() => openField("preset")}>Fix</Button>
                }
              >
                The model preset belongs to another agent.
              </Alert>
            )}
            {missingProject && (
              <Alert
                severity="warning"
                action={
                  <Button onClick={() => openField("project")}>
                    Choose
                  </Button>
                }
              >
                The project is unavailable on the selected Machine.
              </Alert>
            )}
            <Box
              sx={{
                overflowY: fill ? "auto" : undefined,
                minHeight: 0,
                flex: 1,
                display: "grid",
                gridTemplateColumns: desktop
                  ? "minmax(0, 1fr) minmax(0, 1fr)"
                  : "minmax(0, 1fr)",
                gap: desktop ? 2 : 0,
                alignContent: "start",
              }}
            >
              {([{ title: "Workspace", keys: ["project", "machine"] }, {
                title: "Agent",
                keys: ["provider", "preset", "runtime_machine"],
              }, {
                title: "Code browsing",
                keys: ["review_directory"],
              }] as const).map((group) => (
                <Box key={group.title}>
                  <Typography variant="overline" color="text.secondary">
                    {group.title}
                  </Typography>
                  <List disablePadding>
                    {group.keys.map((key) => (
                      <ListItemButton
                        key={key}
                        data-default-field={key}
                        onClick={() => openField(key)}
                        sx={{ px: 1, py: 0.75, borderRadius: 1 }}
                      >
                        <ListItemText
                          primary={FOLDER_DEFAULT_FIELDS[key]}
                          secondary={`${label(key, effective.values[key])} · ${
                            source(key)
                          }`}
                          slotProps={{
                            secondary: { sx: { overflowWrap: "anywhere" } },
                          }}
                        />
                        <ChevronRight fontSize="small" />
                      </ListItemButton>
                    ))}
                  </List>
                  <Divider sx={{ my: 1 }} />
                </Box>
              ))}
            </Box>
            <Typography variant="caption" color="text.secondary">
              {draft
                ? "Saved when you create this folder."
                : "Changes save automatically and sync across devices."}
            </Typography>
          </>
        )}
    </Stack>
  );
}
