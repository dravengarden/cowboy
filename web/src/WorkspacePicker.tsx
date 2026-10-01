import { useMemo, useState } from "react";
import {
  Box,
  Breadcrumbs,
  Button,
  Checkbox,
  FormControlLabel,
  MenuItem,
  MenuList,
  Popover,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { ChevronRight, ExpandMore } from "@mui/icons-material";
import {
  workspaceBranch,
  type WorkspaceEntry,
  workspaceTree,
} from "./workspaceHierarchy";

const preferenceKey = "cowboy.workspaceHierarchy";

export function WorkspacePicker({ entries, value, onChange }: {
  entries: readonly WorkspaceEntry[];
  value: string;
  onChange: (value: string) => void;
}): React.JSX.Element {
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
  const hasGroups = entries.some((entry) => entry.label.includes("/"));
  const query = search.trim().toLocaleLowerCase();
  const grouped = hierarchical && hasGroups && !query;
  const navigate = (next: string[]): void => {
    setPath(next);
    requestAnimationFrame(() => {
      document.querySelector<HTMLElement>(
        "#workspace-picker-menu [role=menuitem]",
      )?.focus();
    });
  };
  const choose = (id: string): void => {
    onChange(id);
    setAnchor(null);
  };
  const entryRow = (
    entry: WorkspaceEntry,
    label = entry.label,
  ): React.JSX.Element => (
    <MenuItem
      key={entry.value}
      selected={entry.value === value}
      onClick={() => choose(entry.value)}
      sx={{ minHeight: 44, whiteSpace: "normal", overflowWrap: "anywhere" }}
    >
      {label}
    </MenuItem>
  );
  return (
    <>
      <TextField
        label="Working directory"
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
          setPath([]);
          setSearch("");
          setAnchor(event.currentTarget);
        }}
        onKeyDown={(event) => {
          if (["Enter", " ", "ArrowDown"].includes(event.key)) {
            event.preventDefault();
            event.stopPropagation();
            setPath([]);
            setSearch("");
            setAnchor(event.currentTarget);
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
            },
          },
        }}
      >
        <Stack sx={{ p: 1.5, gap: 1 }}>
          <TextField
            size="small"
            label="Search directories"
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
              label="Group by directory"
              control={
                <Checkbox
                  size="small"
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
            <Breadcrumbs aria-label="Directory path">
              <Button size="small" onClick={() => navigate([])}>All</Button>
              {branch.path.map((part, index) => (
                <Button
                  key={index}
                  size="small"
                  onClick={() => navigate(branch.path.slice(0, index + 1))}
                >
                  {part}
                </Button>
              ))}
            </Breadcrumbs>
          )}
        </Stack>
        <MenuList
          id="workspace-picker-menu"
          aria-label="Working directories"
          onKeyDown={(event) => {
            if (event.key === "Enter") event.stopPropagation();
            if (event.key === "ArrowLeft" && grouped && branch.path.length) {
              event.preventDefault();
              navigate(branch.path.slice(0, -1));
            }
          }}
        >
          {grouped
            ? (
              <>
                {branch.entries.map((entry) =>
                  entryRow(entry, `Select this directory · ${entry.label}`)
                )}
                {[...branch.children.values()].map((child) =>
                  child.children.size > 0 || child.entries.length !== 1
                    ? (
                      <MenuItem
                        key={JSON.stringify(child.path)}
                        onClick={() => navigate(child.path)}
                        onKeyDown={(event) => {
                          if (event.key === "ArrowRight") {
                            event.preventDefault();
                            navigate(child.path);
                          }
                        }}
                        sx={{ minHeight: 44, gap: 1 }}
                      >
                        <Box
                          sx={{
                            flex: 1,
                            overflowWrap: "anywhere",
                            whiteSpace: "normal",
                          }}
                        >
                          {child.label}
                        </Box>
                        <ChevronRight />
                      </MenuItem>
                    )
                    : entryRow(child.entries[0]!, child.label)
                )}
              </>
            )
            : entries.filter((entry) =>
              entry.label.toLocaleLowerCase().includes(query)
            ).map((entry) => entryRow(entry))}
        </MenuList>
        {query && !entries.some((entry) =>
          entry.label.toLocaleLowerCase().includes(query)
        ) &&
          (
            <Typography sx={{ p: 2 }} color="text.secondary">
              No matching directories
            </Typography>
          )}
      </Popover>
    </>
  );
}
