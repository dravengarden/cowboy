import {
  Alert,
  Box,
  Button,
  Chip,
  Collapse,
  FormControlLabel,
  MenuItem,
  Stack,
  Switch,
  TextField,
  Typography,
} from "@mui/material";
import { useRef, useState, useSyncExternalStore } from "react";
import { editorPluginHost } from "./appHost";
import type { EditorPluginPermission, EditorPluginSetting } from "./manifest";
import type { EditorPluginInstallPlan, EditorPluginView } from "./registry";

const PERMISSION_TEXT: Record<EditorPluginPermission, string> = {
  "editor:read": "Read the text and selection of the editor you run it in",
  "editor:write": "Replace the selection of that editor (undoable)",
};

export function useEditorPlugins() {
  const host = editorPluginHost();
  return useSyncExternalStore(
    host.subscribe,
    host.getSnapshot,
    host.getSnapshot,
  );
}

/** Installed plugins: install from a package file, review permissions,
 * enable, configure, upgrade (with automatic rollback) and uninstall. */
export function EditorPluginManager(): React.JSX.Element {
  const host = editorPluginHost();
  const snapshot = useEditorPlugins();
  const input = useRef<HTMLInputElement>(null);
  const [plan, setPlan] = useState<EditorPluginInstallPlan | null>(null);
  const [message, setMessage] = useState<
    { severity: "error" | "success" | "warning"; text: string } | null
  >(null);
  const [busy, setBusy] = useState(false);
  const act = async (task: () => Promise<void>): Promise<void> => {
    setBusy(true);
    try {
      await task();
    } catch (error) {
      setMessage({
        severity: "error",
        text: error instanceof Error
          ? error.message
          : "The plugin action failed",
      });
    } finally {
      setBusy(false);
    }
  };
  const pick = (file: File | undefined): void => {
    if (!file) return;
    setMessage(null);
    void act(async () => setPlan(await host.inspect(await file.text())));
  };
  return (
    <Stack spacing={1.25} data-editor-plugin-manager>
      <Stack direction="row" alignItems="center" spacing={1}>
        <Typography variant="subtitle2" sx={{ flex: 1 }}>
          Installed plugins
        </Typography>
        <Button
          size="small"
          variant="outlined"
          disabled={busy}
          onClick={() => input.current?.click()}
          data-editor-plugin-install
        >
          Install plugin…
        </Button>
        <input
          ref={input}
          type="file"
          accept=".cowboy-plugin,application/json"
          hidden
          data-editor-plugin-file
          onChange={(event) => {
            pick(event.currentTarget.files?.[0]);
            event.currentTarget.value = "";
          }}
        />
      </Stack>
      {message && (
        <Alert
          severity={message.severity}
          onClose={() => setMessage(null)}
        >
          {message.text}
        </Alert>
      )}
      {plan && (
        <InstallReview
          plan={plan}
          busy={busy}
          onCancel={() => setPlan(null)}
          onConfirm={() =>
            void act(async () => {
              const result = await host.install(plan);
              setPlan(null);
              setMessage({
                severity: result.ok ? "success" : "warning",
                text: result.message,
              });
            })}
        />
      )}
      {snapshot.plugins.length === 0 && !plan && (
        <Typography variant="body2" color="text.secondary">
          No plugins yet. Install a .cowboy-plugin package; plugins run isolated
          from the network and the rest of Cowboy.
        </Typography>
      )}
      {snapshot.plugins.map((plugin) => (
        <InstalledPlugin
          key={plugin.manifest.id}
          plugin={plugin}
          busy={busy}
          act={act}
        />
      ))}
    </Stack>
  );
}

function InstallReview(
  { plan, busy, onCancel, onConfirm }: {
    plan: EditorPluginInstallPlan;
    busy: boolean;
    onCancel: () => void;
    onConfirm: () => void;
  },
): React.JSX.Element {
  const { manifest } = plan.pkg;
  const verb = plan.kind === "install"
    ? "Install"
    : plan.kind === "upgrade"
    ? `Upgrade from ${plan.installedVersion}`
    : plan.kind === "downgrade"
    ? `Downgrade from ${plan.installedVersion}`
    : "Reinstall";
  return (
    <Box
      data-editor-plugin-review
      sx={{ border: 1, borderColor: "divider", borderRadius: 1, p: 1.5 }}
    >
      <Typography variant="subtitle2">
        {manifest.name} {manifest.version}
      </Typography>
      <Typography variant="caption" color="text.secondary" display="block">
        {manifest.author} · {manifest.id} · {plan.pkg.digest.slice(0, 19)}…
      </Typography>
      <Typography variant="body2" sx={{ my: 1 }}>
        {manifest.description}
      </Typography>
      {plan.incompatibility
        ? (
          <Alert severity="error">
            This Cowboy cannot run it: {plan.incompatibility}
          </Alert>
        )
        : (
          <>
            <Typography variant="caption" color="text.secondary">
              Runs in: {manifest.contexts.map((
                c,
              ) => (c === "document" ? "Drafts" : "Sessions")).join(", ")}
              {" · "}
              {manifest.surfaces.map((
                s,
              ) => (s === "desktop" ? "Desktop" : "Touch")).join(", ")}
            </Typography>
            <Box component="ul" sx={{ pl: 2.5, my: 1 }}>
              {manifest.permissions.length === 0 && (
                <li>
                  <Typography variant="body2">No editor access</Typography>
                </li>
              )}
              {manifest.permissions.map((permission) => (
                <li key={permission}>
                  <Typography variant="body2">
                    {PERMISSION_TEXT[permission]}
                    {plan.kind !== "install" &&
                      plan.newPermissions.includes(permission) &&
                      (
                        <Chip
                          size="small"
                          color="warning"
                          label="New"
                          sx={{ ml: 1 }}
                        />
                      )}
                  </Typography>
                </li>
              ))}
            </Box>
            <Typography
              variant="caption"
              color="text.secondary"
              display="block"
            >
              No network, cookies, other documents or Cowboy data. Only install
              packages you trust.
            </Typography>
          </>
        )}
      <Stack direction="row" spacing={1} sx={{ mt: 1.5 }}>
        <Button
          variant="contained"
          size="small"
          disabled={busy || !!plan.incompatibility}
          onClick={onConfirm}
          data-editor-plugin-confirm
        >
          {verb}
        </Button>
        <Button size="small" color="inherit" onClick={onCancel}>Cancel</Button>
      </Stack>
    </Box>
  );
}

function InstalledPlugin(
  { plugin, busy, act }: {
    plugin: EditorPluginView;
    busy: boolean;
    act: (task: () => Promise<void>) => Promise<void>;
  },
): React.JSX.Element {
  const host = editorPluginHost();
  const [open, setOpen] = useState(false);
  const id = plugin.manifest.id;
  const status = plugin.status === "running"
    ? null
    : plugin.status === "starting"
    ? "Starting…"
    : plugin.status === "failed"
    ? "Failed"
    : "Off";
  return (
    <Box data-editor-plugin={id} data-editor-plugin-status={plugin.status}>
      <Stack direction="row" alignItems="center" spacing={1}>
        <FormControlLabel
          sx={{ flex: 1, minWidth: 0, mr: 0 }}
          label={
            <Box sx={{ minWidth: 0 }}>
              <Typography variant="body2" noWrap>
                {plugin.manifest.name}{" "}
                <Typography
                  component="span"
                  variant="caption"
                  color="text.secondary"
                >
                  {plugin.manifest.version}
                </Typography>
                {status && (
                  <Chip
                    size="small"
                    label={status}
                    color={plugin.status === "failed" ? "error" : "default"}
                    sx={{ ml: 1 }}
                  />
                )}
              </Typography>
            </Box>
          }
          control={
            <Switch
              checked={plugin.enabled}
              disabled={busy || plugin.status === "starting"}
              inputProps={{ "aria-label": `Enable ${plugin.manifest.name}` }}
              onChange={(_, enabled) =>
                void act(() => host.setEnabled(id, enabled))}
            />
          }
        />
        <Button
          size="small"
          color="inherit"
          onClick={() => setOpen(!open)}
          aria-expanded={open}
        >
          {open ? "Less" : "Manage"}
        </Button>
      </Stack>
      <Typography variant="caption" color="text.secondary" display="block">
        {plugin.manifest.description}
      </Typography>
      {plugin.failure && (
        <Alert
          severity="error"
          sx={{ mt: 0.5 }}
          action={
            <Button
              color="inherit"
              size="small"
              disabled={busy}
              onClick={() => void act(() => host.setEnabled(id, true))}
            >
              Retry
            </Button>
          }
        >
          {plugin.failure}
        </Alert>
      )}
      <Collapse in={open} unmountOnExit>
        <Stack spacing={1.25} sx={{ pt: 1, pl: 1 }}>
          {plugin.manifest.settings.map((setting) => (
            <PluginSetting
              key={setting.id}
              setting={setting}
              value={plugin.settings[setting.id]!}
              onChange={(value) =>
                void act(() =>
                  host.updateSettings(id, { [setting.id]: value })
                )}
            />
          ))}
          <Typography variant="caption" color="text.secondary">
            {plugin.manifest.permissions.length === 0
              ? "No editor access"
              : plugin.manifest.permissions.map((p) => PERMISSION_TEXT[p]).join(
                " · ",
              )}
            {plugin.previousVersion
              ? ` · Rollback copy: ${plugin.previousVersion}`
              : ""}
          </Typography>
          <Box>
            <Button
              size="small"
              color="error"
              disabled={busy}
              onClick={() => void act(() => host.uninstall(id))}
              data-editor-plugin-uninstall
            >
              Uninstall
            </Button>
          </Box>
        </Stack>
      </Collapse>
    </Box>
  );
}

function PluginSetting(
  { setting, value, onChange }: {
    setting: EditorPluginSetting;
    value: boolean | string | number;
    onChange: (value: boolean | string | number) => void;
  },
): React.JSX.Element {
  const [draft, setDraft] = useState(String(value));
  const helper = setting.description;
  switch (setting.type) {
    case "boolean":
      return (
        <FormControlLabel
          label={setting.title}
          control={
            <Switch
              checked={value === true}
              onChange={(_, v) => onChange(v)}
            />
          }
        />
      );
    case "select":
      return (
        <TextField
          select
          size="small"
          label={setting.title}
          helperText={helper}
          value={String(value)}
          onChange={(e) => onChange(e.target.value)}
        >
          {setting.options.map((o) => (
            <MenuItem key={o.value} value={o.value}>{o.label}</MenuItem>
          ))}
        </TextField>
      );
    case "number":
      return (
        <TextField
          size="small"
          type="number"
          label={setting.title}
          helperText={helper ?? `${setting.min}–${setting.max}`}
          value={draft}
          inputProps={{ min: setting.min, max: setting.max }}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => {
            const n = Number(draft);
            if (
              Number.isFinite(n) && n >= setting.min && n <= setting.max
            ) onChange(n);
            else setDraft(String(value));
          }}
        />
      );
    case "string":
      return (
        <TextField
          size="small"
          label={setting.title}
          helperText={helper}
          value={draft}
          inputProps={{ maxLength: setting.maxLength }}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => {
            if (draft !== value) onChange(draft);
          }}
        />
      );
  }
}
