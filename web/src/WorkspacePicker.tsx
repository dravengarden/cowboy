import { isImeKeyEvent } from "./imeKey";
import { useMemo, useState } from "react";
import {
  Box,
  Breadcrumbs,
  Button,
  Checkbox,
  FormControlLabel,
  InputAdornment,
  MenuItem,
  MenuList,
  Popover,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import {
  Check,
  ChevronRight,
  ExpandMore,
  FolderOpenOutlined,
  FolderOutlined,
  Search,
} from "@mui/icons-material";
import {
  workspaceBranch,
  type WorkspaceEntry,
  workspaceTree,
} from "./workspaceHierarchy";

export function WorkspacePicker(
  { entries, value, onChange, label = "Working directory" }: {
    label?: string;
    entries: readonly WorkspaceEntry[];
    value: string;
    onChange: (value: string) => void;
  },
): React.JSX.Element {
  // Project placement is shared by local and remote execution. Its display
  // preference must not inherit a flat directory picker from an older flow.
  const preferenceKey = label === "Project"
    ? "cowboy.projectHierarchy"
    : "cowboy.workspaceHierarchy";
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const [path, setPath] = useState<string[]>([]);
  const [search, setSearch] = useState("");
  const [hierarchical, setHierarchical] = useState(() => {
    try {
      return localStorage.getItem(preferenceKey) !== "false";
    } catch {
      return true;
    }
  });
  const root = useMemo(() => workspaceTree(entries), [entries]);
  const branch = workspaceBranch(root, path);
  const selected = entries.find((entry) => entry.value === value);
  const hasGroups = entries.some((entry) =>
    (entry.hierarchyPath?.length ?? entry.label.split("/").length) > 1
  );
  const query = search.trim().toLocaleLowerCase();
  const grouped = hierarchical && hasGroups && !query;
  const matches = entries.filter((entry) =>
    `${entry.label}\n${entry.help}`.toLocaleLowerCase().includes(query)
  );
  const navigate = (next: string[]): void => {
    setPath(next);
    requestAnimationFrame(() => {
      document.querySelector<HTMLElement>(
        "#workspace-picker-menu [role=menuitem]",
      )?.focus();
    });
  };
  const openPicker = (element: HTMLElement): void => {
    const selectedPath = selected?.hierarchyPath ??
      selected?.label.split("/") ?? [];
    const node = workspaceBranch(root, selectedPath);
    // A selectable parent opens itself; a leaf opens its containing directory.
    setPath(node.children.size > 0 ? node.path : node.path.slice(0, -1));
    setSearch("");
    setAnchor(element);
  };
  const choose = (id: string): void => {
    onChange(id);
    setAnchor(null);
  };
  const entryRow = (
    entry: WorkspaceEntry,
    rowLabel = entry.label,
    browsePath?: string[],
    currentParent = false,
  ): React.JSX.Element => (
    <MenuItem
      key={entry.value}
      selected={entry.value === value}
      data-current-directory={currentParent || undefined}
      onClick={() => {
        if (browsePath) navigate(browsePath);
        else choose(entry.value);
      }}
      onKeyDown={(event) => {
        if (event.key === "ArrowRight" && browsePath) {
          event.preventDefault();
          navigate(browsePath);
        }
      }}
      sx={{
        gridColumn: "1 / -1",
        minHeight: 44,
        gap: 1.25,
        borderRadius: 1.5,
        whiteSpace: "normal",
        overflowWrap: "anywhere",
        ...(currentParent
          ? {
            border: 1,
            borderColor: "divider",
            bgcolor: "action.hover",
            mb: 1,
            "&.Mui-selected": { bgcolor: "action.selected" },
          }
          : {}),
      }}
    >
      {currentParent
        ? <FolderOpenOutlined fontSize="small" color="primary" />
        : <FolderOutlined fontSize="small" sx={{ color: "text.secondary" }} />}
      <Box sx={{ flex: 1, minWidth: 0 }}>
        <Typography
          component="span"
          sx={{ fontWeight: currentParent ? 600 : 400 }}
        >
          {rowLabel}
        </Typography>
        {currentParent && (
          <Typography variant="caption" display="block" color="text.secondary">
            {label === "Project" ? "Select this project" : "Use this directory"}
          </Typography>
        )}
      </Box>
      {entry.value === value && <Check fontSize="small" color="primary" />}
      {browsePath && (
        <ChevronRight fontSize="small" sx={{ color: "text.secondary" }} />
      )}
    </MenuItem>
  );
  return (
    <>
      <TextField
        label={label}
        value={selected?.label ?? ""}
        helperText={selected?.help ?? ""}
        slotProps={{
          input: {
            readOnly: true,
            endAdornment: <ExpandMore />,
          },
          htmlInput: {
            role: "combobox",
            "aria-expanded": Boolean(anchor),
            "aria-haspopup": "menu",
            "aria-controls": anchor ? "workspace-picker-menu" : undefined,
            style: { cursor: "pointer" },
          },
        }}
        onClick={(event) => {
          openPicker(event.currentTarget);
        }}
        onKeyDown={(event) => {
          if (isImeKeyEvent(event.nativeEvent)) return;
          if (["Enter", " ", "ArrowDown"].includes(event.key)) {
            event.preventDefault();
            event.stopPropagation();
            openPicker(event.currentTarget);
          }
        }}
      />
      <Popover
        open={Boolean(anchor)}
        anchorEl={anchor}
        onClose={() => setAnchor(null)}
        anchorOrigin={{ vertical: "bottom", horizontal: "left" }}
        slotProps={{
          paper: {
            sx: {
              width: anchor?.clientWidth,
              maxWidth: "calc(100vw - 32px)",
              maxHeight: "min(70dvh, 600px)",
              display: "flex",
              flexDirection: "column",
              border: 1,
              borderColor: "divider",
            },
          },
        }}
      >
        <Stack sx={{ p: 1.5, pb: 0.5, gap: 0.5, flexShrink: 0 }}>
          <TextField
            size="small"
            label={label === "Project"
              ? "Search projects"
              : "Search directories"}
            slotProps={{
              input: {
                startAdornment: (
                  <InputAdornment position="start">
                    <Search fontSize="small" />
                  </InputAdornment>
                ),
              },
            }}
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "ArrowDown") {
                event.preventDefault();
                document.querySelector<HTMLElement>(
                  "#workspace-picker-menu [role=menuitem]",
                )?.focus();
              }
              if (event.key === "Enter") event.stopPropagation();
            }}
          />
          {hasGroups && (
            <FormControlLabel
              label={label === "Project"
                ? "Group by machine and directory"
                : "Group by directory"}
              sx={{
                m: 0,
                alignSelf: "flex-start",
                "& .MuiFormControlLabel-label": {
                  fontSize: "0.875rem",
                  color: "text.secondary",
                },
              }}
              control={
                <Checkbox
                  size="small"
                  sx={{ p: 1.25, ml: -1.25 }}
                  checked={hierarchical}
                  onChange={(_, checked) => {
                    setHierarchical(checked);
                    setPath([]);
                    try {
                      localStorage.setItem(preferenceKey, String(checked));
                    } catch { /* optional preference */ }
                  }}
                />
              }
            />
          )}
          {grouped && branch.path.length > 0 && (
            <Breadcrumbs
              aria-label={label === "Project"
                ? "Project path"
                : "Directory path"}
              separator={
                <ChevronRight sx={{ fontSize: 16, color: "text.secondary" }} />
              }
              sx={{
                px: 0.5,
                bgcolor: "action.hover",
                borderRadius: 1.5,
                "& .MuiBreadcrumbs-separator": { mx: 0.25 },
                "& .MuiBreadcrumbs-li": { minWidth: 0, maxWidth: "100%" },
                "& .MuiButton-root": {
                  minWidth: 0,
                  minHeight: 44,
                  px: 1,
                  textTransform: "none",
                  fontSize: "0.875rem",
                  overflowWrap: "anywhere",
                },
              }}
            >
              <Button size="small" onClick={() => navigate([])}>
                {label === "Project" ? "All projects" : "All directories"}
              </Button>
              {branch.path.map((part, index) =>
                index === branch.path.length - 1
                  ? (
                    <Typography
                      key={index}
                      aria-current="location"
                      sx={{
                        px: 1,
                        py: 1.25,
                        fontSize: "0.875rem",
                        fontWeight: 600,
                        overflowWrap: "anywhere",
                      }}
                    >
                      {part}
                    </Typography>
                  )
                  : (
                    <Button
                      key={index}
                      size="small"
                      onClick={() => navigate(branch.path.slice(0, index + 1))}
                    >
                      {part}
                    </Button>
                  )
              )}
            </Breadcrumbs>
          )}
        </Stack>
        <MenuList
          id="workspace-picker-menu"
          autoFocusItem
          aria-label={label === "Project" ? "Projects" : "Working directories"}
          sx={{
            px: 0.75,
            pb: 0.75,
            overflowY: "auto",
            minHeight: 0,
            display: "grid",
            gridTemplateColumns: "minmax(0, 1fr)",
            gridAutoRows: "minmax(44px, auto)",
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.stopPropagation();
            if (event.key === "ArrowLeft" && grouped && branch.path.length) {
              event.preventDefault();
              navigate(branch.path.slice(0, -1));
            }
          }}
        >
          {grouped
            ? [
              ...branch.entries.map((entry) =>
                entryRow(entry, branch.label, undefined, true)
              ),
              ...[...branch.children.values()].flatMap((child) => {
                if (child.entries.length === 1) {
                  const hasChildren = child.children.size > 0;
                  return [
                    entryRow(
                      child.entries[0]!,
                      child.label,
                      hasChildren ? child.path : undefined,
                    ),
                  ];
                }
                return [
                  <MenuItem
                    key={`group:${JSON.stringify(child.path)}`}
                    onClick={() => navigate(child.path)}
                    onKeyDown={(event) => {
                      if (event.key === "ArrowRight") {
                        event.preventDefault();
                        navigate(child.path);
                      }
                    }}
                    sx={{
                      gridColumn: "1 / -1",
                      minHeight: 44,
                      gap: 1.25,
                      borderRadius: 1.5,
                    }}
                  >
                    <FolderOutlined
                      fontSize="small"
                      sx={{ color: "text.secondary" }}
                    />
                    <Box
                      sx={{
                        flex: 1,
                        overflowWrap: "anywhere",
                        whiteSpace: "normal",
                      }}
                    >
                      {child.label}
                    </Box>
                    <ChevronRight
                      fontSize="small"
                      sx={{ color: "text.secondary" }}
                    />
                  </MenuItem>,
                ];
              }),
            ]
            : matches.map((entry) => entryRow(entry))}
        </MenuList>
        {query && matches.length === 0 &&
          (
            <Typography sx={{ p: 2 }} color="text.secondary">
              {label === "Project"
                ? "No matching projects"
                : "No matching directories"}
            </Typography>
          )}
      </Popover>
    </>
  );
}
