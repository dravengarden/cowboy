import { Refresh } from "@mui/icons-material";
import {
  Alert,
  Button,
  CircularProgress,
  IconButton,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useState } from "react";
import {
  compareProviderVersions,
  validateMachinePluginInventory,
} from "@cowboy/provider-ui";
import { ConfirmSheet } from "../Sheet";
import { useStoreSelector } from "../store";
import { createPluginInstallRequest } from "../pluginInstallation";
import type { PluginRelease, PluginRemovalPlan } from "../admin/adminApi";

async function json<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    cache: "no-store",
    credentials: "same-origin",
    ...init,
  });
  if (!response.ok) {
    await response.body?.cancel();
    throw new Error(
      "The operation could not be completed. Refresh installation status before trying again.",
    );
  }
  return response.json();
}

/** Uses the existing exact-release installer and one-use removal plans. */
export function ExtensionManager(
  { initialMachineId }: { initialMachineId?: string | undefined } = {},
): React.JSX.Element {
  const machines = useStoreSelector((snapshot) => snapshot.machines);
  const [machineId, setMachineId] = useState(
    initialMachineId ?? machines[0]?.id ?? "",
  );
  const machine = machines.find((m) => m.id === machineId);
  let installedPlugins: ReturnType<typeof validateMachinePluginInventory>;
  try {
    installedPlugins = validateMachinePluginInventory(machine?.plugins ?? []);
  } catch {
    installedPlugins = [];
  }
  const [releases, setReleases] = useState<PluginRelease[]>([]);
  const [refresh, setRefresh] = useState(0);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<Record<string, string>>({});
  const [removal, setRemoval] = useState<PluginRemovalPlan | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    void json<{ plugins: PluginRelease[] }>("/api/plugins", {
      signal: controller.signal,
    }).then((result) => {
      if (!controller.signal.aborted) {
        setReleases(
          result.plugins.filter((p) => p.plugin_kind === "workspace_extension"),
        );
      }
    }).catch(() => {
      if (!controller.signal.aborted) {
        setError("Extension catalog unavailable.");
      }
    }).finally(() => {
      if (!controller.signal.aborted) setLoading(false);
    });
    return () => controller.abort();
  }, [refresh]);
  async function run(action: () => Promise<unknown>, success: string) {
    setBusy(true);
    setError("");
    setMessage("");
    try {
      await action();
      setMessage(success);
      setRefresh((v) => v + 1);
    } catch (error) {
      setError(
        error instanceof Error
          ? error.message
          : "Extension operation unavailable",
      );
    } finally {
      setBusy(false);
    }
  }
  const ids = [
    ...new Set([
      ...releases.map((r) => r.plugin_id),
      ...installedPlugins.filter((p) => p.plugin_kind === "workspace_extension")
        .map((p) => p.plugin_id),
    ]),
  ].sort();
  return (
    <Stack spacing={2}>
      <Stack direction="row" alignItems="center">
        <Typography component="h2" variant="h6" sx={{ flex: 1 }}>
          Manage extensions
        </Typography>
        <IconButton
          disabled={busy}
          aria-label="Refresh extension catalog"
          onClick={() => setRefresh((v) => v + 1)}
        >
          <Refresh />
        </IconButton>
      </Stack>
      <Typography variant="body2" color="text.secondary">
        Install repository tools on the Machine where your workspace lives. Each
        extension uses the same Plugin installation and update controls.
      </Typography>
      <TextField
        select
        label="Machine"
        size="small"
        value={machineId}
        disabled={busy}
        onChange={(e) => {
          setMachineId(e.target.value);
          setRemoval(null);
          setError("");
          setMessage("");
        }}
      >
        {machines.map((m) => (
          <MenuItem key={m.id} value={m.id}>{m.display_name || m.id}</MenuItem>
        ))}
      </TextField>
      <TextField
        label="Find an extension"
        size="small"
        value={search}
        onChange={(e) => setSearch(e.target.value)}
      />
      {error && <Alert severity="warning">{error}</Alert>}
      {message && <Alert severity="success">{message}</Alert>}
      {loading
        ? <CircularProgress size={24} />
        : !ids.length
        ? (
          <Typography color="text.secondary">
            No extensions are available in the catalog yet.
          </Typography>
        )
        : ids.filter((id) => id.includes(search.toLowerCase())).map((id) => {
          const versions = releases.filter((p) =>
            p.plugin_id === id && p.release_state === "ready" &&
            p.artifact_digest
          ).sort((a, b) =>
            compareProviderVersions(b.plugin_version, a.plugin_version)
          );
          const release = versions.find((p) =>
            p.artifact_digest === selected[id]
          ) ?? versions[0];
          const installed = installedPlugins.find((p) => p.plugin_id === id);
          const current = installed?.state === "active" &&
            installed.generation_digest === release?.artifact_digest;
          const supported = release?.supported_platforms.some((p) =>
            p.os === machine?.platform &&
            p.architecture === machine?.architecture
          ) &&
            (machine?.plugin_contracts?.max_release_schema ?? 0) >=
              (release?.compatibility_requirements?.release_schema ?? 3);
          return (
            <Stack
              key={id}
              spacing={1}
              sx={{
                p: 1.5,
                border: 1,
                borderColor: "divider",
                borderRadius: 2,
              }}
            >
              <Stack direction="row" alignItems="center" spacing={1}>
                <Typography fontWeight={650} sx={{ flex: 1 }}>{id}</Typography>
                <Typography variant="caption" color="text.secondary">
                  {installed
                    ? `${installed.plugin_version} · ${installed.state}`
                    : "Not installed"}
                </Typography>
              </Stack>
              {release
                ? (
                  <TextField
                    select
                    size="small"
                    label="Available version"
                    value={release.artifact_digest}
                    disabled={busy}
                    onChange={(e) =>
                      setSelected((values) => ({
                        ...values,
                        [id]: e.target.value,
                      }))}
                  >
                    {versions.map((v) => (
                      <MenuItem
                        key={v.artifact_digest}
                        value={v.artifact_digest!}
                      >
                        {v.plugin_version}
                      </MenuItem>
                    ))}
                  </TextField>
                )
                : (
                  <Typography variant="body2" color="text.secondary">
                    A signed release is not available yet.
                  </Typography>
                )}
              {release && !supported && (
                <Typography variant="caption" color="text.secondary">
                  Update Cowboy Machine to a compatible version to install this
                  extension.
                </Typography>
              )}
              <Stack direction="row" spacing={1}>
                <Button
                  disabled={busy || !release || !supported || current ||
                    machine?.status !== "online"}
                  onClick={() => {
                    if (!release?.artifact_digest) return;
                    const request = createPluginInstallRequest(
                      release.plugin_version,
                      release.artifact_digest,
                    );
                    void run(
                      () =>
                        json(
                          `/api/machines/${
                            encodeURIComponent(machineId)
                          }/plugins/${encodeURIComponent(id)}`,
                          {
                            method: "POST",
                            headers: { "content-type": "application/json" },
                            body: JSON.stringify(request),
                          },
                        ),
                      "Extension installed. Return to Extensions and refresh.",
                    );
                  }}
                >
                  {current
                    ? "Installed"
                    : installed
                    ? "Change version"
                    : "Install"}
                </Button>
                {installed && (
                  <Button
                    color="error"
                    disabled={busy || machine?.status !== "online"}
                    onClick={() => {
                      void run(async () => {
                        const plan = await json<PluginRemovalPlan>(
                          `/api/machines/${
                            encodeURIComponent(machineId)
                          }/plugins/${encodeURIComponent(id)}/uninstall-plan`,
                          { method: "POST" },
                        );
                        setRemoval(plan);
                      }, "");
                    }}
                  >
                    Uninstall
                  </Button>
                )}
              </Stack>
            </Stack>
          );
        })}
      <ConfirmSheet
        open={removal !== null}
        onClose={() => {
          if (!busy) setRemoval(null);
        }}
        title="Uninstall extension"
        actions={
          <>
            <Button disabled={busy} onClick={() => setRemoval(null)}>
              Cancel
            </Button>
            <Button
              color="error"
              disabled={busy || !removal ||
                removal.affected_sessions.length > 0}
              onClick={() => {
                if (!removal) return;
                const plan = removal;
                setRemoval(null);
                void run(() =>
                  json(
                    `/api/machines/${
                      encodeURIComponent(plan.machine_id)
                    }/plugins/${encodeURIComponent(plan.plugin_id)}/uninstall`,
                    {
                      method: "POST",
                      headers: { "content-type": "application/json" },
                      body: JSON.stringify({
                        plan_id: plan.plan_id,
                        confirm_active_sessions: false,
                      }),
                    },
                  ), "Extension uninstalled.");
              }}
            >
              Uninstall
            </Button>
          </>
        }
      >
        <Typography>
          {removal?.warning ||
            "The extension will be removed from this Machine."}
        </Typography>
        {removal && removal.affected_sessions.length > 0 && (
          <Alert severity="warning">
            This extension is in use. Close affected sessions before
            uninstalling.
          </Alert>
        )}
      </ConfirmSheet>
    </Stack>
  );
}
