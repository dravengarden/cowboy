import { useId, useRef } from "react";
import {
  Box,
  ListItemButton,
  ListItemText,
  Stack,
  Typography,
} from "@mui/material";
import { ExpandLess, ExpandMore, Tune } from "@mui/icons-material";
import { FolderDefaultsEditor } from "./FolderDefaultsEditor";
import {
  FOLDER_DEFAULT_FIELDS,
  type FolderDefaultKey,
  type FolderDefaults,
  resolveFolderDefaults,
} from "./folderDefaults";
import { reviewDirectoryLabel } from "./reviewDirectoryDefault";
import { sessionFolderById, type SessionFoldersValue } from "./sessionFolders";
import { directoryProjects } from "./directoryProjects";
import {
  latestProviderEntries,
  type useProviderCatalog,
} from "./providerCatalog";
import type { MachineSummary } from "./protocol";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { useStoreSelector } from "./store";

/** The create draft never enters the shared store until the one create mutation. */
export function FolderCreationDefaults({
  name,
  parent,
  defaults,
  onChange,
  field,
  onFieldChange,
  expanded,
  onExpandedChange,
  tree,
  machines,
  catalog,
  error,
}: {
  name: string;
  parent: string | null;
  defaults: FolderDefaults;
  onChange: (defaults: FolderDefaults) => void;
  field: FolderDefaultKey | null;
  onFieldChange: (field: FolderDefaultKey | null) => void;
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
  tree: SessionFoldersValue;
  machines: readonly MachineSummary[];
  catalog: ReturnType<typeof useProviderCatalog>["catalog"];
  error?: string;
}): React.JSX.Element {
  const id = useId();
  const sessions = useStoreSelector((state) => state.sessions);
  const desktop = useSurfaceProfile().kind === "desktop";
  const section = useRef<HTMLDivElement>(null);
  const preview: SessionFoldersValue = {
    ...tree,
    folders: [...tree.folders, {
      id,
      name: name.trim() || "New folder",
      parent,
      position: 0,
      project: null,
      defaults,
    }],
  };
  const effective = resolveFolderDefaults(preview, id).values;
  const entries = latestProviderEntries(catalog?.providers ?? []);
  const provider = effective.provider ?? "claude-code";
  const entry = entries.find((candidate) => candidate.provider_id === provider);
  const projects = directoryProjects(machines, effective.machine);
  const project = projects.find((p) => p.value === effective.project);
  const machineName = (value: string | null | undefined) =>
    value && (machines.find((m) => m.id === value)?.display_name ?? value);
  const preset = entry?.manifest.configuration.presets.find((p) =>
    `${provider}/${p.id}` === effective.preset
  );
  const summary = [
    project?.label ?? effective.project ?? "Choose project when creating",
    machineName(effective.machine),
    entry?.manifest.display.name ??
      (provider === "claude-code" ? "Claude" : provider),
    preset?.name ?? effective.preset,
    effective.runtime_machine
      ? `Agent on ${machineName(effective.runtime_machine)}`
      : null,
    effective.review_directory !== undefined
      ? `Zed: ${
        reviewDirectoryLabel(effective.review_directory, machines, sessions)
      }`
      : null,
  ].filter(Boolean).join(" · ");
  const count = Object.keys(defaults).length;
  const source = parent
    ? sessionFolderById(tree, parent)?.name ?? "parent directory"
    : null;
  const inheritance = source
    ? `Inherits from ${source}`
    : "Uses global defaults";
  return (
    <Stack ref={section} spacing={1} data-folder-creation-defaults>
      {!field && (
        <ListItemButton
          data-folder-defaults-toggle
          aria-expanded={expanded}
          aria-controls={id}
          onClick={() => {
            onExpandedChange(!expanded);
            if (!expanded && !desktop) {
              requestAnimationFrame(() =>
                section.current?.scrollIntoView({ block: "nearest" })
              );
            }
          }}
          sx={{
            px: 1.25,
            py: 0.75,
            border: "1px solid",
            borderColor: "divider",
            borderRadius: 1.5,
            gap: 1.25,
          }}
        >
          <Tune
            fontSize="small"
            sx={{ color: "text.secondary", flexShrink: 0 }}
          />
          <ListItemText
            primary={
              <Stack
                direction="row"
                alignItems="baseline"
                spacing={1}
                flexWrap="wrap"
              >
                <Typography variant="body2" fontWeight={600}>
                  Session defaults
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  {count
                    ? `${count} override${count === 1 ? "" : "s"}`
                    : "Optional"}
                </Typography>
              </Stack>
            }
            secondary={
              <>
                {!expanded && (
                  <Box
                    component="span"
                    sx={{ display: "block", overflowWrap: "anywhere" }}
                  >
                    {summary}
                  </Box>
                )}
                <Box component="span" sx={{ display: "block" }}>
                  {count === Object.keys(FOLDER_DEFAULT_FIELDS).length
                    ? "All settings chosen for this folder"
                    : count
                    ? `Other settings: ${inheritance.toLowerCase()}`
                    : inheritance}
                </Box>
              </>
            }
          />
          {expanded
            ? <ExpandLess fontSize="small" />
            : <ExpandMore fontSize="small" />}
        </ListItemButton>
      )}
      {(expanded || field) && (
        <Box id={id}>
          <FolderDefaultsEditor
            id={id}
            tree={preview}
            machines={machines}
            catalog={catalog}
            error={error ?? null}
            field={field}
            onFieldChange={onFieldChange}
            draft
            onSetDefault={(_id, key, value) => {
              const next = { ...defaults };
              if (value === undefined) delete next[key];
              else next[key] = value;
              onChange(next);
            }}
          />
        </Box>
      )}
    </Stack>
  );
}
