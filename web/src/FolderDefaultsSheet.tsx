import {
  Alert,
  Button,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { setSessionFolderDefault, useStoreSelector } from "./store";
import {
  FOLDER_DEFAULT_FIELDS,
  type FolderDefaultKey,
  resolveFolderDefaults,
} from "./folderDefaults";
import { sessionFolderById } from "./sessionFolders";
import { latestProviderEntries, useProviderCatalog } from "./providerCatalog";
import { InAppRoot } from "./SessionFolderUi";
import { Sheet } from "./Sheet";
import { useNavbarAtBottom } from "./navbarSettings";
import { useRef } from "react";
import { useDialogFocus } from "./useDialogInputFocus";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { Kbd } from "./Kbd";

export function FolderDefaultsSheet(
  { id, onClose }: { id: string; onClose: () => void },
): React.JSX.Element {
  const tree = useStoreSelector((state) => state.sessionFolders);
  const machines = useStoreSelector((state) => state.machines);
  const folder = sessionFolderById(tree, id);
  const { catalog, error } = useProviderCatalog();
  const entries = latestProviderEntries(catalog?.providers ?? []);
  const bottom = useNavbarAtBottom();
  const content = useRef<HTMLDivElement>(null);
  const desktop = useSurfaceProfile().kind === "desktop";
  useDialogFocus(
    () => content.current?.querySelector<HTMLElement>('[role="combobox"]'),
    desktop,
  );
  const inherited = resolveFolderDefaults(tree, folder?.parent ?? null);
  const effective = resolveFolderDefaults(tree, id);
  const projects = [
    ...new Map(
      machines.flatMap((machine) =>
        machine.workspaces.map((project) =>
          [project.id, {
            value: project.id,
            label: project.display_name,
          }] as const
        )
      ),
    ).values(),
  ];
  const machineOptions = machines.map((machine) => ({
    value: machine.id,
    label: `${machine.display_name}${machine.connected ? "" : " · Offline"}`,
  }));
  const choices: Record<FolderDefaultKey, { value: string; label: string }[]> =
    {
      project: projects,
      machine: machineOptions,
      runtime_machine: machineOptions,
      provider: entries.map((entry) => ({
        value: entry.provider_id,
        label: entry.manifest.display.name,
      })),
      preset: entries.filter((entry) =>
        !effective.values.provider ||
        entry.provider_id === effective.values.provider
      ).flatMap((entry) =>
        entry.manifest.configuration.presets.map((preset) => ({
          value: `${entry.provider_id}/${preset.id}`,
          label:
            `${entry.manifest.display.name} · ${preset.name} · ${preset.detail}`,
        }))
      ),
    };
  return (
    <InAppRoot>
      <Sheet
        open
        forceSheet={bottom}
        onClose={onClose}
        title={`${folder?.name ?? "Directory"} defaults`}
        mobileDismiss="none"
        actions={
          <Button onClick={onClose}>
            Done <Kbd keys="Esc" />
          </Button>
        }
      >
        <Stack ref={content} spacing={2} sx={{ py: 1 }}>
          <Typography variant="body2" color="text.secondary">
            Applies to new sessions and inherited by subdirectories. Existing
            sessions keep their settings. Changes save automatically.
          </Typography>
          {error && <Alert severity="error">{error}</Alert>}
          {!folder
            ? <Alert severity="error">This directory was removed.</Alert>
            : (Object.keys(FOLDER_DEFAULT_FIELDS) as FolderDefaultKey[]).map(
              (key) => {
                const own = folder.defaults?.[key];
                const value = own === undefined
                  ? "inherit"
                  : own === null
                  ? "none"
                  : `value:${own}`;
                const options = choices[key];
                const source = effective.sources[key];
                const current = effective.values[key];
                const label =
                  options.find((option) => option.value === current)?.label ??
                    current;
                return (
                  <TextField
                    key={key}
                    select
                    fullWidth
                    label={FOLDER_DEFAULT_FIELDS[key]}
                    value={value}
                    helperText={source
                      ? `${label ?? "Global default / no binding"} · ${
                        source === id
                          ? "This directory"
                          : `Inherited from ${
                            sessionFolderById(tree, source)?.name ?? source
                          }`
                      }`
                      : "Uses the New Session default"}
                    onChange={(event) =>
                      setSessionFolderDefault(
                        id,
                        key,
                        event.target.value === "inherit"
                          ? undefined
                          : event.target.value === "none"
                          ? null
                          : event.target.value.slice(6),
                      )}
                  >
                    <MenuItem value="inherit">
                      Inherit{inherited.values[key]
                        ? ` · ${
                          options.find((option) =>
                            option.value === inherited.values[key]
                          )?.label ?? inherited.values[key]
                        }`
                        : ""}
                    </MenuItem>
                    <MenuItem value="none">
                      {key === "project"
                        ? "No project — choose when creating"
                        : "Use global default"}
                    </MenuItem>
                    {own && !options.some((option) => option.value === own) && (
                      <MenuItem value={`value:${own}`}>
                        {own} · Unavailable or incompatible
                      </MenuItem>
                    )}
                    {options.map((option) => (
                      <MenuItem
                        key={option.value}
                        value={`value:${option.value}`}
                      >
                        {option.label}
                      </MenuItem>
                    ))}
                  </TextField>
                );
              },
            )}
        </Stack>
      </Sheet>
    </InAppRoot>
  );
}
