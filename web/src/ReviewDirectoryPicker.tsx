import { useState } from "react";
import {
  Box,
  IconButton,
  InputAdornment,
  List,
  ListItemButton,
  ListItemText,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { Check, ChevronRight, ExpandMore, Search } from "@mui/icons-material";
import type { MachineSummary, SessionMeta } from "./protocol";
import { directoryProjects } from "./directoryProjects";
import { reviewDirectoryLabel } from "./reviewDirectoryDefault";
import {
  registeredReviewWorkspace,
  sessionReviewLocation,
  workspaceCodeContextId,
} from "./mobile/review/reviewContextModel";
import { sessionExecution } from "./sessionExecution";
import { useSurfaceProfile } from "./surface/SurfaceProfile";

/** One picker for directory creation, directory defaults and session overrides. */
export function ReviewDirectoryPicker(
  { value, inherited, machines, sessions = [], onChange }: {
    value: string | null | undefined;
    inherited: string | null | undefined;
    machines: readonly MachineSummary[];
    sessions?: readonly SessionMeta[];
    onChange: (value: string | null | undefined) => void;
  },
): React.JSX.Element {
  const desktop = useSurfaceProfile().kind === "desktop";
  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState(new Set<string>());
  const search = query.trim().toLocaleLowerCase();
  const mode = (
    id: string,
    title: string,
    description: string,
    selected: boolean,
    next: string | null | undefined,
  ) => (
    <ListItemButton
      data-review-directory={id}
      selected={selected}
      onClick={() => onChange(next)}
      sx={{
        borderRadius: 1,
        minHeight: desktop ? 44 : 52,
        px: 1,
        py: 0.25,
        gap: 1,
      }}
    >
      <ListItemText
        primary={title}
        secondary={description}
        slotProps={{
          primary: { variant: "body2", fontWeight: 600 },
          secondary: { sx: { fontSize: "0.75rem" } },
        }}
      />
      {selected && <Check color="primary" fontSize="small" />}
    </ListItemButton>
  );
  const matches = (...texts: string[]) =>
    !search || texts.join(" ").toLocaleLowerCase().includes(search);
  let count = 0;
  const choice = (
    id: string,
    name: string,
    detail: string,
    depth = 0,
    branch = false,
  ) => {
    count++;
    const expanded = !collapsed.has(id);
    return (
      <Box
        key={id}
        sx={{
          display: "flex",
          ml: depth * 2,
          borderLeft: depth ? "1px solid" : undefined,
          borderColor: "divider",
        }}
      >
        {branch && (
          <IconButton
            aria-label={`${expanded ? "Collapse" : "Expand"} ${name}`}
            aria-expanded={expanded}
            sx={{
              width: desktop ? 32 : 44,
              height: desktop ? 40 : 48,
              flexShrink: 0,
            }}
            onClick={() =>
              setCollapsed((previous) => {
                const next = new Set(previous);
                if (expanded) next.add(id);
                else next.delete(id);
                return next;
              })}
          >
            {expanded
              ? <ExpandMore fontSize="small" />
              : <ChevronRight fontSize="small" />}
          </IconButton>
        )}
        <ListItemButton
          selected={value === id}
          data-review-directory={id}
          onClick={() => onChange(id)}
          sx={{
            minWidth: 0,
            minHeight: desktop ? 40 : 48,
            px: 1,
            py: 0.5,
            gap: 1,
            borderRadius: 1,
          }}
        >
          <ListItemText
            primary={name}
            secondary={detail}
            slotProps={{
              primary: { variant: "body2" },
              secondary: {
                sx: { overflowWrap: "anywhere", fontSize: "0.7rem" },
              },
            }}
          />
          {value === id && <Check fontSize="small" color="primary" />}
        </ListItemButton>
      </Box>
    );
  };
  return (
    <Stack
      spacing={1}
      sx={{ minHeight: 0, flex: 1 }}
      data-review-directory-picker
    >
      <Stack spacing={0.25}>
        {mode(
          "inherit",
          "Inherit",
          reviewDirectoryLabel(inherited, machines, sessions),
          value === undefined,
          undefined,
        )}
        {mode(
          "automatic",
          "Automatic",
          "This session’s worktree, then project",
          value === null,
          null,
        )}
        {mode(
          "project",
          "Associated project",
          "Main checkout of this session’s project",
          value === "project",
          "project",
        )}
      </Stack>
      <TextField
        size="small"
        autoFocus={desktop}
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder="Find a project or worktree"
        slotProps={{
          htmlInput: { "aria-label": "Find a Zed directory" },
          input: {
            startAdornment: (
              <InputAdornment position="start">
                <Search fontSize="small" />
              </InputAdornment>
            ),
          },
        }}
      />
      <Box
        sx={{
          overflowY: "auto",
          minHeight: 0,
          flex: 1,
          maxHeight: "min(42dvh, 420px)",
          overscrollBehavior: "contain",
        }}
      >
        {machines.map((machine) => {
          const projects = directoryProjects([machine]);
          const nodes: React.ReactNode[] = [];
          const used = new Set<string>();
          const appendSession = (session: SessionMeta, depth: number) => {
            const execution = sessionExecution(session);
            const registered = registeredReviewWorkspace(
              session,
              machine.workspaces.map((w) => ({
                id: w.id,
                displayName: w.display_name,
                canonicalPath: w.canonical_path,
              })),
            );
            if (matches(session.title, execution.cwd, machine.display_name)) {
              nodes.push(
                choice(
                  `session::${session.id}`,
                  session.title,
                  `${
                    sessionReviewLocation(session, registered).kind
                  } · ${execution.cwd}`,
                  search ? 0 : depth,
                ),
              );
            }
          };
          const eligible = sessions.filter((session) => {
            const e = sessionExecution(session);
            return (e.state === "ready" || e.state === "local") && e.cwd &&
              (e.machineId ? e.machineId === machine.id : machine.local);
          });
          const append = (parent: string | null, depth: number) => {
            for (const project of projects.filter((p) => p.parent === parent)) {
              const identity = workspaceCodeContextId(
                machine.id,
                project.value,
              );
              const children = eligible.filter((session) =>
                registeredReviewWorkspace(
                  session,
                  machine.workspaces.map((w) => ({
                    id: w.id,
                    displayName: w.display_name,
                    canonicalPath: w.canonical_path,
                  })),
                )?.id === project.value
              );
              children.forEach((s) => used.add(s.id));
              const branch = children.length > 0 ||
                projects.some((p) => p.parent === project.value);
              const path = machine.workspaces.find((w) =>
                w.id === project.value
              )!.canonical_path;
              if (matches(project.label, path, machine.display_name)) {
                nodes.push(
                  choice(
                    identity,
                    project.label,
                    path,
                    search ? 0 : depth,
                    branch && !search,
                  ),
                );
              }
              if (search || !collapsed.has(identity)) {
                children.forEach((s) => appendSession(s, depth + 1));
                append(project.value, depth + 1);
              }
            }
          };
          append(null, 0);
          // Worktrees without a registered source are still valid session scopes.
          eligible.filter((s) =>
            !used.has(s.id) &&
            !registeredReviewWorkspace(
              s,
              machine.workspaces.map((w) => ({
                id: w.id,
                displayName: w.display_name,
                canonicalPath: w.canonical_path,
              })),
            )
          ).forEach((s) => appendSession(s, 0));
          return nodes.length
            ? (
              <Box key={machine.id}>
                <Typography
                  variant="overline"
                  color="text.secondary"
                  sx={{ px: 1 }}
                >
                  {machine.display_name}
                  {machine.connected ? "" : " · Offline"}
                </Typography>
                <List
                  disablePadding
                  aria-label={`${machine.display_name} code directories`}
                >
                  {nodes}
                </List>
              </Box>
            )
            : null;
        })}
        {!count && (
          <Typography variant="body2" color="text.secondary" sx={{ p: 1 }}>
            No matching directories.
          </Typography>
        )}
      </Box>
    </Stack>
  );
}
