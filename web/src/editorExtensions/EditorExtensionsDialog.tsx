import {
  Alert,
  Box,
  Button,
  Divider,
  FormControlLabel,
  List,
  ListItemButton,
  ListItemText,
  Stack,
  Switch,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { documentNotice } from "../documents/DocumentNotifications";
import { Sheet } from "../Sheet";
import { SegmentedTabs } from "../SegmentedTabs";
import { productSyncPrincipal } from "../productSyncIdentity";
import { useSurfaceProfile } from "../surface/SurfaceProfile";
import {
  useDesktopCommand,
  useDesktopCommands,
} from "../desktop/commands/DesktopCommandProvider";
import {
  DEFAULT_EDITOR_TEMPLATES,
  type EditorTemplate,
  outlineExtension,
  templateExtension,
} from "./builtins";
import {
  activeEditorExtensionPort,
  closeEditorExtensions,
  editorExtensionDialog,
  openEditorExtensions,
} from "./host";
import { createEditorExtensionRuntime } from "./runtime";
import type { EditorPanelItem } from "./contract";
import {
  editorPluginCommandId,
  editorPluginHost,
  loadEditorPlugins,
} from "../editorPlugins/appHost";
import {
  EditorPluginManager,
  useEditorPlugins,
} from "../editorPlugins/EditorPluginManager";

interface Settings {
  templates: readonly EditorTemplate[];
  disabled: readonly string[];
}
function settingsKey(): string {
  return `cowboy:editor-extensions:${productSyncPrincipal() ?? "local"}:v1`;
}
function readSettings(): Settings {
  try {
    const value = JSON.parse(localStorage.getItem(settingsKey()) ?? "null") as
      | Settings
      | null;
    if (
      value && Array.isArray(value.templates) && value.templates.length <= 30 &&
      Array.isArray(value.disabled) &&
      value.templates.every((t) =>
        typeof t.id === "string" && /^[a-z][a-z0-9-]{0,63}$/.test(t.id) &&
        typeof t.title === "string" && typeof t.text === "string"
      ) &&
      new Set(value.templates.map((t) => t.id)).size === value.templates.length
    ) return value;
  } catch { /* An invalid preference cannot prevent editing. */ }
  return { templates: DEFAULT_EDITOR_TEMPLATES, disabled: [] };
}

export function EditorExtensionsCommand(): null {
  const registry = useDesktopCommands();
  const [settings, setSettings] = useState(readSettings);
  useEffect(() => {
    const refresh = (): void => setSettings(readSettings());
    globalThis.addEventListener("cowboy:editor-extensions-changed", refresh);
    return () =>
      globalThis.removeEventListener(
        "cowboy:editor-extensions-changed",
        refresh,
      );
  }, []);
  useEffect(() => {
    if (settings.disabled.includes("cowboy-templates")) return undefined;
    const disposers = settings.templates.map((template) =>
      registry.register({
        id: `editor.template.${template.id}`,
        title: `Templates: ${template.title}`,
        group: "Editing",
        run: () => {
          const port = activeEditorExtensionPort();
          if (!port) return;
          const runtime = createEditorExtensionRuntime(port);
          try {
            runtime.activate(templateExtension([template]));
            void Promise.resolve(runtime.commands()[0]?.run(port)).catch((
              error: Error,
            ) => documentNotice(error.message)).finally(() =>
              runtime.dispose()
            );
          } catch (error) {
            runtime.dispose();
            documentNotice(
              error instanceof Error ? error.message : "Template failed",
            );
          }
        },
      })
    );
    return () => {
      for (const dispose of disposers) dispose();
    };
  }, [registry.register, settings]);
  const plugins = useEditorPlugins();
  useEffect(() => {
    loadEditorPlugins();
  }, []);
  useEffect(() => {
    const disposers = plugins.plugins.flatMap((plugin) =>
      plugin.commands.map((command) =>
        registry.register({
          id: editorPluginCommandId(plugin.manifest.id, command.id),
          title: `${plugin.manifest.name}: ${command.title}`,
          ...(command.description ? { description: command.description } : {}),
          group: "Plugins",
          allowInEditor: true,
          when: () => {
            const port = activeEditorExtensionPort();
            return !!port && editorPluginHost().appliesTo(plugin.manifest.id, port);
          },
          disabledReason: "Focus a Draft or Session editor this plugin supports",
          run: () => runEditorPluginCommand(plugin.manifest.id, command.id),
        })
      )
    );
    return () => {
      for (const dispose of disposers) dispose();
    };
  }, [registry.register, plugins]);
  const command = useMemo(
    () => ({
      id: "editor.extensions",
      title: "Editor extensions",
      group: "Editing",
      description: "Templates, outline and custom editor tools",
      run: () => openEditorExtensions(),
    }),
    [],
  );
  useDesktopCommand(command);
  return null;
}

/** The single execution path for palette, toolbar and the Tools list. */
export function runEditorPluginCommand(plugin: string, command: string): void {
  const port = activeEditorExtensionPort();
  if (!port) {
    documentNotice("Focus a Draft or Session editor first.");
    return;
  }
  void editorPluginHost().runCommand(plugin, command, port).catch((error: unknown) =>
    documentNotice(error instanceof Error ? error.message : "The plugin command failed")
  );
}

export function EditorExtensionsDialog(): React.JSX.Element | null {
  const binding = useSyncExternalStore(
    editorExtensionDialog.subscribe,
    editorExtensionDialog.get,
    editorExtensionDialog.get,
  );
  return binding ? <ExtensionWorkbench binding={binding} /> : null;
}

function ExtensionWorkbench(
  { binding }: {
    binding: NonNullable<ReturnType<typeof editorExtensionDialog.get>>;
  },
): React.JSX.Element {
  const [settings, setSettings] = useState(readSettings);
  const [tab, setTab] = useState<"tools" | "extensions">("tools");
  const [error, setError] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState(() => binding.port.read());
  const [editing, setEditing] = useState<EditorTemplate | null>(null);
  const desktop = useSurfaceProfile().kind === "desktop";
  const plugins = useEditorPlugins();
  useEffect(() => {
    loadEditorPlugins();
  }, []);
  const pluginHost = editorPluginHost();
  const applicable = plugins.plugins.filter((plugin) =>
    plugin.status === "running" &&
    pluginHost.appliesTo(plugin.manifest.id, binding.port)
  );
  const [pluginPanels, setPluginPanels] = useState<
    Record<string, readonly EditorPanelItem[] | string>
  >({});
  useEffect(() => {
    if (tab !== "tools") return undefined;
    let current = true;
    for (const plugin of applicable) {
      for (const panel of plugin.panels) {
        const key = `${plugin.manifest.id}:${panel.id}`;
        void pluginHost.renderPanel(plugin.manifest.id, panel.id, binding.port).then(
          (items) => current && setPluginPanels((all) => ({ ...all, [key]: items })),
          (cause: unknown) =>
            current &&
            setPluginPanels((all) => ({
              ...all,
              [key]: cause instanceof Error ? cause.message : "Panel failed",
            })),
        );
      }
    }
    return () => {
      current = false;
    };
    // Re-render plugin panels when the document revision or plugins change.
  }, [tab, plugins, snapshot.revision, binding]);
  const extensions = useMemo(
    () => [templateExtension(settings.templates), outlineExtension],
    [settings.templates],
  );
  const [runtime, setRuntime] = useState<
    ReturnType<typeof createEditorExtensionRuntime> | null
  >(null);
  useEffect(() => {
    const host = createEditorExtensionRuntime(binding.port);
    for (const extension of extensions) {
      if (settings.disabled.includes(extension.id)) continue;
      try {
        host.activate(extension);
      } catch (cause) {
        setError(
          `${extension.title}: ${
            cause instanceof Error ? cause.message : "Could not activate"
          }`,
        );
      }
    }
    setRuntime(host);
    return () => host.dispose();
  }, [binding, extensions, settings.disabled]);
  useEffect(() => {
    const timer = globalThis.setInterval(() => {
      if (binding.alive()) setSnapshot(binding.port.read());
    }, 400);
    return () => globalThis.clearInterval(timer);
  }, [binding]);
  const update = (next: Settings): void => {
    try {
      localStorage.setItem(settingsKey(), JSON.stringify(next));
      setSettings(next);
      globalThis.dispatchEvent(new Event("cowboy:editor-extensions-changed"));
    } catch {
      setError("Could not save extension settings on this device.");
    }
  };
  const run = async (
    command: import("./contract").EditorExtensionCommand,
  ): Promise<void> => {
    try {
      await command.run(binding.port);
      closeEditorExtensions();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Extension failed");
    }
  };
  return (
    <Sheet
      open
      title="Editor extensions"
      forceSheet={!desktop}
      onClose={closeEditorExtensions}
      actions={<Button onClick={closeEditorExtensions}>Done</Button>}
    >
      <SegmentedTabs
        value={tab}
        onChange={setTab}
        aria-label="Editor extension tools"
        options={[
          { value: "tools", label: "Tools" },
          { value: "extensions", label: "Extensions" },
        ]}
      />
      {error && (
        <Alert severity="error" onClose={() => setError(null)}>{error}</Alert>
      )}
      <Box sx={{ maxHeight: "55dvh", overflowY: "auto", minWidth: 0 }}>
        {tab === "tools"
          ? (
            <>
              <List dense>
                {(runtime?.commands() ?? []).map((command) => (
                  <ListItemButton
                    key={command.id}
                    onClick={() => void run(command)}
                  >
                    <ListItemText
                      primary={command.title}
                      secondary={command.description}
                    />
                  </ListItemButton>
                ))}
              </List>
              {applicable.length > 0 && (
                <List dense data-editor-plugin-tools>
                  {applicable.flatMap((plugin) =>
                    plugin.commands.map((command) => (
                      <ListItemButton
                        key={`${plugin.manifest.id}:${command.id}`}
                        onClick={() => {
                          closeEditorExtensions();
                          void pluginHost.runCommand(plugin.manifest.id, command.id, binding.port)
                            .catch((cause: unknown) =>
                              documentNotice(
                                cause instanceof Error ? cause.message : "The plugin command failed",
                              )
                            );
                        }}
                      >
                        <ListItemText
                          primary={`${plugin.manifest.name}: ${command.title}`}
                          secondary={command.description}
                        />
                      </ListItemButton>
                    ))
                  )}
                </List>
              )}
              {applicable.flatMap((plugin) =>
                plugin.panels.map((panel) => {
                  const key = `${plugin.manifest.id}:${panel.id}`;
                  const items = pluginPanels[key];
                  return (
                    <Box key={key} sx={{ py: 1 }} data-editor-plugin-panel={key}>
                      <Typography variant="subtitle2">
                        {plugin.manifest.name}: {panel.title}
                      </Typography>
                      {typeof items === "string"
                        ? <Typography variant="caption" color="error">{items}</Typography>
                        : (
                          <List dense>
                            {(items ?? []).map((item) => (
                              <ListItemButton
                                key={item.id}
                                disabled={item.offset === undefined}
                                sx={{ pl: 1 + (item.depth ?? 0) * 1.5 }}
                                onClick={() => {
                                  closeEditorExtensions();
                                  binding.port.reveal(item.offset!);
                                }}
                              >
                                <ListItemText primary={item.label} secondary={item.detail} />
                              </ListItemButton>
                            ))}
                          </List>
                        )}
                    </Box>
                  );
                })
              )}
              {(runtime?.panels() ?? []).map((panel) => (
                <Box key={panel.id} sx={{ py: 1 }}>
                  <Typography variant="subtitle2">{panel.title}</Typography>
                  <List dense>
                    {panel.read(snapshot).map((item) => (
                      <ListItemButton
                        key={item.id}
                        disabled={item.offset === undefined}
                        sx={{ pl: 1 + (item.depth ?? 0) * 1.5 }}
                        onClick={() => {
                          closeEditorExtensions();
                          binding.port.reveal(item.offset!);
                        }}
                      >
                        <ListItemText
                          primary={item.label}
                          secondary={item.detail}
                        />
                      </ListItemButton>
                    ))}
                  </List>
                </Box>
              ))}
            </>
          )
          : (
            <Stack spacing={1.5} sx={{ py: 1 }}>
              <Typography variant="body2" color="text.secondary">
                Shared by Drafts and Session editors. Settings apply on this
                device.
              </Typography>
              {extensions.map((extension) => (
                <Box key={extension.id}>
                  <FormControlLabel
                    label={extension.title}
                    control={
                      <Switch
                        checked={!settings.disabled.includes(extension.id)}
                        onChange={(_, enabled) =>
                          update({
                            ...settings,
                            disabled: enabled
                              ? settings.disabled.filter((id) =>
                                id !== extension.id
                              )
                              : [...settings.disabled, extension.id],
                          })}
                      />
                    }
                  />
                  <Typography
                    variant="caption"
                    display="block"
                    color="text.secondary"
                  >
                    {extension.description}
                  </Typography>
                </Box>
              ))}
              <Divider />
              <EditorPluginManager />
              <Divider />
              <Typography variant="subtitle2">Your templates</Typography>
              {settings.templates.map((template) => (
                <Button
                  key={template.id}
                  color="inherit"
                  onClick={() => setEditing(template)}
                  sx={{ justifyContent: "flex-start" }}
                >
                  {template.title}
                </Button>
              ))}
              <Button
                disabled={settings.templates.length >= 30}
                onClick={() =>
                  setEditing({
                    id: `t-${crypto.randomUUID()}`,
                    title: "",
                    text: "",
                  })}
              >
                New template
              </Button>
              {editing && (
                <Stack spacing={1}>
                  <TextField
                    label="Template name"
                    value={editing.title}
                    onChange={(e) =>
                      setEditing({ ...editing, title: e.target.value })}
                    inputProps={{ maxLength: 160 }}
                  />
                  <TextField
                    label="Markdown"
                    multiline
                    minRows={4}
                    value={editing.text}
                    onChange={(e) =>
                      setEditing({ ...editing, text: e.target.value })}
                    helperText="{{selection}} inserts selected text; {{date}} inserts today's date."
                  />
                  <Stack direction="row" spacing={1}>
                    <Button
                      disabled={!editing.title.trim() ||
                        editing.text.length > 100000}
                      onClick={() => {
                        update({
                          ...settings,
                          templates: [
                            ...settings.templates.filter((t) =>
                              t.id !== editing.id
                            ),
                            { ...editing, title: editing.title.trim() },
                          ],
                        });
                        setEditing(null);
                      }}
                    >
                      Save template
                    </Button>
                    <Button color="inherit" onClick={() => setEditing(null)}>
                      Cancel
                    </Button>
                    <Button
                      color="error"
                      onClick={() => {
                        update({
                          ...settings,
                          templates: settings.templates.filter((t) =>
                            t.id !== editing.id
                          ),
                        });
                        setEditing(null);
                      }}
                    >
                      Remove
                    </Button>
                  </Stack>
                </Stack>
              )}
            </Stack>
          )}
      </Box>
    </Sheet>
  );
}
