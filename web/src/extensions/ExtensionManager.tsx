import { desktopSize } from "../surface/desktopSize";
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
import { useEffect, useRef, useState } from "react";
import {
  compareProviderVersions,
  genericPluginCompatibilityProblem,
  validateMachinePluginInventory,
} from "@cowboy/provider-ui";
import { ConfirmSheet } from "../Sheet";
import { useStoreSelector } from "../store";
import { createPluginInstallRequest } from "../pluginInstallation";
import type { PluginRelease, PluginRemovalPlan } from "../admin/adminApi";
import type { MachineSummary } from "../protocol";
import { PluginLifecycleHistory } from "../PluginLifecycleHistory";
import {
  changeExtensionInstallation,
  extensionManagementJson as json,
} from "./managementApi";

/** Uses the existing exact-release installer and one-use removal plans. */
export function ExtensionManager(
  { initialMachineId }: { initialMachineId?: string | undefined } = {},
): React.JSX.Element {
  const machines = useStoreSelector((snapshot) => snapshot.machines);
  return (
    <ExtensionManagerView
      machines={machines}
      initialMachineId={initialMachineId}
    />
  );
}

/** The shared management surface consumes the live Machine inventory. */
export function ExtensionManagerView(
  { machines, initialMachineId }: {
    machines: readonly MachineSummary[];
    initialMachineId?: string | undefined;
  },
): React.JSX.Element {
  const firstMachineId = machines[0]?.id;
  const [selectedMachineId, setMachineId] = useState(
    initialMachineId ?? firstMachineId,
  );
  const machineId = selectedMachineId ?? firstMachineId ?? "";
  useEffect(() => {
    if (selectedMachineId === undefined && firstMachineId !== undefined) {
      setMachineId(firstMachineId);
    }
  }, [firstMachineId, selectedMachineId]);
  const machine = machines.find((m) => m.id === machineId);
  let installedPlugins: ReturnType<typeof validateMachinePluginInventory>;
  let inventoryUnavailable = false;
  try {
    installedPlugins = validateMachinePluginInventory(machine?.plugins ?? []);
  } catch {
    installedPlugins = [];
    inventoryUnavailable = true;
  }
  const [releases, setReleases] = useState<PluginRelease[]>([]);
  const [refresh, setRefresh] = useState(0);
  const [loading, setLoading] = useState(true);
  const [catalogError, setCatalogError] = useState("");
  const [busy, setBusy] = useState(false);
  const pendingAction = useRef(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<Record<string, string>>({});
  const [removal, setRemoval] = useState<PluginRemovalPlan | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setCatalogError("");
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
        setCatalogError("Extension catalog unavailable. Refresh to try again.");
      }
    }).finally(() => {
      if (!controller.signal.aborted) setLoading(false);
    });
    return () => controller.abort();
  }, [refresh]);
  async function run(action: () => Promise<unknown>, success: string) {
    if (pendingAction.current) return;
    pendingAction.current = true;
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
      pendingAction.current = false;
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
  const matchingIds = ids.filter((id) =>
    id.includes(search.trim().toLowerCase())
  );
  return (
    <Stack spacing={2}>
      <Stack direction="row" alignItems="center">
        <Typography component="h2" variant="h6" sx={{ flex: 1 }}>
          Manage extensions
        </Typography>
        <IconButton
          disabled={busy || loading}
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
        value={machine?.id ?? ""}
        disabled={busy}
        onChange={(e) => {
          setMachineId(e.target.value);
          setRemoval(null);
          setError("");
          setMessage("");
        }}
      >
        <MenuItem value="" disabled>Select a Machine</MenuItem>
        {machines.map((m) => (
          <MenuItem key={m.id} value={m.id}>
            {m.display_name || m.id} · {m.status}
          </MenuItem>
        ))}
      </TextField>
      {!machine && (
        <Alert severity="info">
          Select a Machine to manage its extensions.
        </Alert>
      )}
      {machine && machine.status !== "online" && (
        <Alert severity="info">
          {machine.display_name || machine.id} is{" "}
          {machine.status}. Installation changes are available when it is
          online.
        </Alert>
      )}
      {inventoryUnavailable && (
        <Alert severity="warning">
          Installed extension status is unavailable. Reconnect this Machine
          before changing extensions.
        </Alert>
      )}
      <TextField
        label="Find an extension"
        size="small"
        value={search}
        onChange={(e) => setSearch(e.target.value)}
      />
      {catalogError && <Alert severity="warning">{catalogError}</Alert>}
      {error && <Alert severity="warning">{error}</Alert>}
      {message && <Alert severity="success">{message}</Alert>}
      {loading && (
        <CircularProgress size={desktopSize(24)} aria-label="Loading extension catalog" />
      )}
      {loading && !ids.length
        ? null
        : !ids.length && catalogError
        ? null
        : !ids.length
        ? (
          <Typography color="text.secondary">
            No extensions are available in the catalog yet.
          </Typography>
        )
        : !matchingIds.length
        ? (
          <Typography color="text.secondary">
            No extensions match your search.
          </Typography>
        )
        : matchingIds.map((id) => {
          const versions = releases.filter((p) =>
            p.plugin_id === id && p.release_state === "ready" &&
            p.artifact_digest
          ).sort((a, b) =>
            compareProviderVersions(b.plugin_version, a.plugin_version)
          );
          const installed = installedPlugins.find((p) => p.plugin_id === id);
          const selectionKey = JSON.stringify([machineId, id]);
          const latestCompatible = machine
            ? versions.find((candidate) =>
              !genericPluginCompatibilityProblem(candidate, machine)
            )
            : undefined;
          // Keep an installed or explicitly selected digest even when it leaves
          // the catalog; refreshing must never silently select another release.
          const selectedDigest = selected[selectionKey] ??
            installed?.generation_digest ??
            latestCompatible?.artifact_digest ?? versions[0]?.artifact_digest;
          const release = versions.find((p) =>
            p.artifact_digest === selectedDigest
          );
          const current = installed?.state === "active" &&
            installed.generation_digest === selectedDigest;
          const problem = machine && release
            ? genericPluginCompatibilityProblem(release, machine)
            : undefined;
          const supported = Boolean(machine && release && !problem);
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
                <Typography
                  fontWeight={650}
                  sx={{ flex: 1, minWidth: 0, overflowWrap: "anywhere" }}
                >
                  {id}
                </Typography>
                <Typography
                  variant="caption"
                  color="text.secondary"
                  sx={{ minWidth: 0, overflowWrap: "anywhere" }}
                >
                  {!machine
                    ? "No Machine selected"
                    : inventoryUnavailable
                    ? "Installation status unavailable"
                    : installed
                    ? `${installed.plugin_version} · ${installed.state}`
                    : "Not installed"}
                </Typography>
              </Stack>
              {versions.length > 0 || selectedDigest
                ? (
                  <TextField
                    select
                    size="small"
                    label="Available version"
                    value={selectedDigest ?? ""}
                    disabled={busy || loading}
                    onChange={(e) =>
                      setSelected((values) => ({
                        ...values,
                        [selectionKey]: e.target.value,
                      }))}
                  >
                    {selectedDigest && !release && (
                      <MenuItem value={selectedDigest} disabled>
                        {installed?.generation_digest === selectedDigest
                          ? `${installed.plugin_version} · Current installation`
                          : "Selected release unavailable"}
                      </MenuItem>
                    )}
                    {versions.map((v) => (
                      <MenuItem
                        key={v.artifact_digest}
                        value={v.artifact_digest!}
                      >
                        {v.plugin_version}
                        {v.artifact_digest === installed?.generation_digest
                          ? " · Current installation"
                          : v.artifact_digest ===
                              latestCompatible?.artifact_digest
                          ? " · Latest compatible"
                          : ""}
                      </MenuItem>
                    ))}
                  </TextField>
                )
                : (
                  <Typography variant="body2" color="text.secondary">
                    A signed release is not available yet.
                  </Typography>
                )}
              {selectedDigest && !release && (
                <Typography variant="caption" color="text.secondary">
                  This exact release is no longer available in the catalog.
                  Select an available version to change the installation.
                </Typography>
              )}
              {installed && latestCompatible &&
                compareProviderVersions(
                    latestCompatible.plugin_version,
                    installed.plugin_version,
                  ) > 0 &&
                (
                  <Typography variant="caption" color="text.secondary">
                    Update available: {latestCompatible.plugin_version}
                  </Typography>
                )}
              {problem && (
                <Typography variant="caption" color="text.secondary">
                  {problem.detail}
                </Typography>
              )}
              <Stack direction="row" spacing={1}>
                <Button
                  disabled={busy || loading || inventoryUnavailable ||
                    !release || !supported || current ||
                    machine?.status !== "online"}
                  onClick={() => {
                    if (!release?.artifact_digest) return;
                    const request = createPluginInstallRequest(
                      release.plugin_version,
                      release.artifact_digest,
                    );
                    void run(
                      () =>
                        changeExtensionInstallation(
                          `/api/machines/${
                            encodeURIComponent(machineId)
                          }/plugins/${encodeURIComponent(id)}`,
                          {
                            method: "POST",
                            headers: { "content-type": "application/json" },
                            body: JSON.stringify(request),
                          },
                        ),
                      "Extension installed.",
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
              {machine && (
                <PluginLifecycleHistory
                  key={selectionKey}
                  machine={machine.id}
                  plugin={id}
                />
              )}
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
                  changeExtensionInstallation(
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
