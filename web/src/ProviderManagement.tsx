import {
  Alert,
  Box,
  Button,
  Checkbox,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  Link,
  Paper,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import {
  type EffectCapability,
  type EffectSchema,
  type MachineProviderInventory,
  type ProviderAuthenticationPresentation,
  type ProviderAuthenticationStatus,
  type ProviderCatalogEntry,
  type ProviderContractInventory,
  type ProviderHostContext,
  type ProviderUiManifest,
  resolveProviderAuthenticationPresentation,
  validateMachineProviderInventory,
} from "../../packages/provider-ui-sdk/src/index.ts";
import {
  joinProviderInstallations,
  latestProviderEntries,
  providerPresentationEntry,
  useProviderCatalog,
} from "./providerCatalog";
import { ProviderMark, ProviderSurface } from "./ProviderSurface";

interface ProviderMachine {
  id: string;
  display_name: string;
  platform: "linux" | "macos";
  architecture: "x86_64" | "aarch64";
  status: string;
  schedulable: boolean;
  provider_contracts?: ProviderContractInventory;
}

interface AffectedSession {
  id: string;
  title: string;
  status: string;
}

interface UninstallPlan {
  plan_id: string;
  machine_id: string;
  provider_id: string;
  provider_version: string;
  generation_digest: string;
  affected_sessions: AffectedSession[];
  active_session_ids: string[];
  purge_after_ms: number;
  expires_at_ms: number;
  warning: string;
}

type LoginEvent =
  | {
    event: "login_challenge";
    request_id: string;
    provider: string;
    verification_url: string;
    user_code?: string;
    input_required?: boolean;
    input_label?: string;
    secret_input?: boolean;
    expires_at_ms: number;
  }
  | {
    event: "login_state";
    request_id: string;
    provider: string;
    state: string;
    account_label?: string;
    detail?: string;
  }
  | {
    event: "command_result";
    request_id: string;
    accepted: boolean;
    detail?: string;
  };

interface AuthenticationFlow {
  provider: ProviderCatalogEntry;
  sharedProviderNames: string[];
  requestId?: string;
  expiresAtMs?: number;
  events: LoginEvent[];
}

const UNPUBLISHED_RELEASE_EFFECTS: ReadonlySet<EffectCapability> = new Set([
  "install_on_machine",
  "upgrade_on_machine",
]);
const UNPUBLISHED_AUTH_EFFECTS: ReadonlySet<EffectCapability> = new Set([
  "begin_service_authentication",
]);

type ProviderManagementProps =
  | { scope: "service"; machine?: never }
  | { scope: "machine"; machine: ProviderMachine };

export function ProviderAuthenticationManagement(): React.JSX.Element {
  return <ProviderManagement scope="service" />;
}

export function MachineProviderManagement(
  { machine }: { machine: ProviderMachine },
): React.JSX.Element {
  return <ProviderManagement scope="machine" machine={machine} />;
}

type ProviderManagementStatusTone = "default" | "success" | "warning";

/** Cowboy owns management-card geometry; Providers supply only typed brand and
 * content slots. This keeps independently authored packages visually stable. */
function ProviderManagementIdentity({
  manifest,
  version,
  statusLabel,
  statusTone,
  actions,
}: {
  manifest: ProviderUiManifest;
  version: string;
  statusLabel: string;
  statusTone: ProviderManagementStatusTone;
  actions: ReactNode;
}): React.JSX.Element {
  return (
    <Box
      data-provider-management-identity
      sx={{
        display: "grid",
        gridTemplateColumns: "32px minmax(0, 1fr)",
        columnGap: 1,
        alignItems: "start",
        minWidth: 0,
      }}
    >
      <Box
        data-provider-management-mark
        sx={{
          width: 32,
          height: 32,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          color: manifest.display.accent,
        }}
      >
        <ProviderMark manifest={manifest} size={28} />
      </Box>
      <Stack spacing={0.45} sx={{ minWidth: 0 }}>
        <Typography
          variant="subtitle1"
          fontWeight={720}
          sx={{
            lineHeight: 1.25,
            overflowWrap: "anywhere",
            display: "-webkit-box",
            WebkitBoxOrient: "vertical",
            WebkitLineClamp: 2,
            overflow: "hidden",
          }}
        >
          {manifest.display.name}
        </Typography>
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{
            lineHeight: 1.35,
            overflowWrap: "anywhere",
            display: "-webkit-box",
            WebkitBoxOrient: "vertical",
            WebkitLineClamp: 2,
            overflow: "hidden",
          }}
        >
          {manifest.display.summary}
        </Typography>
        <Stack
          data-provider-management-footer
          direction="row"
          spacing={0.65}
          alignItems="center"
          flexWrap="wrap"
          useFlexGap
          sx={{ minHeight: 36, pt: 0.15 }}
        >
          <Chip
            data-provider-management-status
            size="small"
            variant="outlined"
            color={statusTone}
            label={statusLabel}
            sx={{
              maxWidth: 138,
              height: 24,
              "& .MuiChip-label": {
                px: 0.8,
                fontSize: "0.68rem",
                overflow: "hidden",
                textOverflow: "ellipsis",
              },
            }}
          />
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ fontFamily: "monospace", lineHeight: 1 }}
          >
            v{version}
          </Typography>
          <Box
            data-provider-management-actions
            sx={{
              ml: "auto",
              minWidth: 0,
              "& > .MuiStack-root": {
                alignItems: "center",
                justifyContent: "flex-end",
              },
              "& .MuiButton-root": { minHeight: 36, px: 1.25 },
            }}
          >
            {actions}
          </Box>
        </Stack>
      </Stack>
    </Box>
  );
}

type ProviderManagementLifecycleSlot = "setup" | "empty" | "settings";

function ProviderManagementLifecycleSurface({
  manifest,
  slot,
  host,
  blockedCapabilities,
  onEffect,
}: {
  manifest: ProviderUiManifest;
  slot: ProviderManagementLifecycleSlot;
  host: ProviderHostContext;
  blockedCapabilities: ReadonlySet<EffectCapability> | undefined;
  onEffect: (effect: EffectSchema) => Promise<void>;
}): React.JSX.Element {
  return (
    <Box sx={{ minWidth: 0 }}>
      <ProviderSurface
        manifest={manifest}
        slot={slot}
        host={host}
        blockedCapabilities={blockedCapabilities}
        onEffect={onEffect}
      />
    </Box>
  );
}

function ProviderManagement(
  { scope, machine }: ProviderManagementProps,
): React.JSX.Element {
  const { catalog, error: catalogError, refresh: refreshCatalog } =
    useProviderCatalog();
  const [inventory, setInventory] = useState<MachineProviderInventory[]>([]);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [flow, setFlow] = useState<AuthenticationFlow | null>(null);
  const [loginInput, setLoginInput] = useState("");
  const [uninstallPlan, setUninstallPlan] = useState<UninstallPlan | null>(
    null,
  );
  const [confirmActive, setConfirmActive] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const machineId = machine?.id;
  const detailsId = scope === "service"
    ? "provider-service-management"
    : `provider-machine-management-${machine?.id ?? "unknown"}`;
  const latestEntries = useMemo(
    () => latestProviderEntries(catalog?.providers ?? []),
    [catalog],
  );
  const providerRows = useMemo(
    () =>
      scope === "machine"
        ? joinProviderInstallations(catalog?.providers ?? [], inventory, {
          platform: machine.platform,
          architecture: machine.architecture,
          ...(machine.provider_contracts
            ? { provider_contracts: machine.provider_contracts }
            : {}),
        })
        : latestEntries.map((latestEntry) => ({
          providerId: latestEntry.provider_id,
          latestEntry,
          latestCompatibleEntry: latestEntry,
          latestCompatibility: undefined,
          installed: undefined,
          installedEntry: undefined,
        })),
    [catalog, inventory, latestEntries, machine, scope],
  );
  const authentications = useMemo(
    () =>
      new Map(
        (catalog?.authentications ?? []).map((
          status,
        ) => [status.provider_id, status]),
      ),
    [catalog],
  );
  const authenticationsByScope = useMemo(
    () =>
      new Map(
        (catalog?.authentications ?? []).map((status) => [
          status.authentication_scope,
          status,
        ]),
      ),
    [catalog],
  );
  const authenticationForEntry = useCallback(
    (entry: ProviderCatalogEntry): ProviderAuthenticationStatus | undefined =>
      authentications.get(entry.provider_id) ??
      authenticationsByScope.get(entry.authentication_scope),
    [authentications, authenticationsByScope],
  );
  const refreshInventory = useCallback(async (): Promise<void> => {
    if (!machineId) {
      setInventory([]);
      return;
    }
    const response = await fetch(
      `/api/machines/${encodeURIComponent(machineId)}/providers`,
    );
    if (!response.ok) {
      throw new Error(
        (await response.text()).trim() || "Could not load installed Providers",
      );
    }
    setInventory(validateMachineProviderInventory(await response.json()));
  }, [machineId]);
  useEffect(() => {
    if (scope !== "machine") return undefined;
    void refreshInventory().catch(() => undefined);
    const timer = globalThis.setInterval(
      () => void refreshInventory().catch(() => undefined),
      2_000,
    );
    return () => globalThis.clearInterval(timer);
  }, [refreshInventory, scope]);
  useEffect(() => {
    if (!flow?.requestId) return undefined;
    let active = true;
    const poll = async (): Promise<void> => {
      try {
        const response = await fetch(
          `/api/providers/${
            encodeURIComponent(flow.provider.provider_id)
          }/auth/${encodeURIComponent(flow.requestId ?? "")}`,
        );
        if (!response.ok) return;
        const body = await response.json() as { events?: LoginEvent[] };
        if (!active || !Array.isArray(body.events)) return;
        setFlow((current) => {
          if (!current || current.requestId !== flow.requestId) return current;
          return { ...current, events: body.events ?? [] };
        });
        const ready = body.events.some((event) =>
          event.event === "login_state" &&
          (event.state === "signed_in" || event.state === "ready")
        );
        if (ready) {
          await refreshCatalog();
          await refreshInventory();
        }
      } catch {
        // A later poll can recover a transient network failure.
      }
    };
    void poll();
    const timer = globalThis.setInterval(() => void poll(), 750);
    return () => {
      active = false;
      globalThis.clearInterval(timer);
    };
  }, [
    flow?.provider.provider_id,
    flow?.requestId,
    refreshCatalog,
    refreshInventory,
  ]);

  const requestUninstallPlan = async (providerId: string): Promise<void> => {
    if (scope !== "machine" || !machine) {
      throw new Error(
        "Machine Provider lifecycle is unavailable from Service authentication",
      );
    }
    const response = await fetch(
      `/api/machines/${encodeURIComponent(machine.id)}/providers/${
        encodeURIComponent(providerId)
      }/uninstall-plan`,
      { method: "POST" },
    );
    await expectSuccess(response, "Could not prepare Provider uninstall");
    setConfirmActive(false);
    setUninstallPlan(await response.json() as UninstallPlan);
  };

  const run = async (
    entry: ProviderCatalogEntry,
    installed: MachineProviderInventory | undefined,
    effect: { capability: EffectCapability },
  ): Promise<void> => {
    setErrors((current) => ({ ...current, [entry.provider_id]: "" }));
    try {
      switch (effect.capability) {
        case "install_on_machine":
        case "upgrade_on_machine": {
          if (scope !== "machine" || !machine) {
            throw new Error(
              "Machine Provider lifecycle is unavailable from Service authentication",
            );
          }
          if (entry.release_state !== "ready" || !entry.artifact_digest) {
            throw new Error(
              entry.release_detail ?? "No signed runtime release is published",
            );
          }
          const response = await fetch(
            `/api/machines/${encodeURIComponent(machine.id)}/providers/${
              encodeURIComponent(entry.provider_id)
            }`,
            {
              method: "POST",
              headers: { "content-type": "application/json" },
              body: JSON.stringify({
                version: entry.provider_version,
                digest: entry.artifact_digest,
              }),
            },
          );
          await expectSuccess(response, "Provider installation failed");
          await refreshInventory();
          return;
        }
        case "begin_service_authentication":
          if (scope !== "service") {
            throw new Error(
              "Provider authentication is managed at Cowboy Service scope",
            );
          }
          setFlow({
            provider: entry,
            sharedProviderNames: latestEntries
              .filter((candidate) =>
                candidate.authentication_scope === entry.authentication_scope
              )
              .map((candidate) => candidate.manifest.display.name),
            events: [],
          });
          return;
        case "logout_service_authentication": {
          if (scope !== "service") {
            throw new Error(
              "Provider authentication is managed at Cowboy Service scope",
            );
          }
          const copy = authenticationCopy(
            resolveProviderAuthenticationPresentation(
              entry.manifest.authentication,
            ),
          );
          const response = await fetch(
            `/api/providers/${encodeURIComponent(entry.provider_id)}/auth`,
            {
              method: "DELETE",
            },
          );
          await expectSuccess(response, copy.clearFailed);
          await refreshCatalog();
          await refreshInventory();
          return;
        }
        case "request_uninstall_plan": {
          if (!installed) {
            throw new Error("Provider is not installed on this Machine");
          }
          await requestUninstallPlan(entry.provider_id);
          return;
        }
        case "open_external_documentation":
          throw new Error(
            "This Provider did not supply a documentation target",
          );
        default:
          return assertUnhandled(effect.capability);
      }
    } catch (cause) {
      const detail = cause instanceof Error
        ? cause.message
        : "Provider operation failed";
      setErrors((current) => ({ ...current, [entry.provider_id]: detail }));
      throw cause;
    }
  };

  const startAuthentication = async (method: string): Promise<void> => {
    if (!flow) return;
    if (!flow.provider.artifact_digest) {
      throw new Error(
        "Provider authentication requires a signed runtime release",
      );
    }
    const response = await fetch(
      `/api/providers/${
        encodeURIComponent(flow.provider.provider_id)
      }/auth/start`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          method,
          provider_version: flow.provider.provider_version,
          generation_digest: flow.provider.artifact_digest,
        }),
      },
    );
    await expectSuccess(response, "Could not start Provider authentication");
    const body = await response.json() as {
      request_id: string;
      expires_at_ms: number;
    };
    setFlow({
      ...flow,
      requestId: body.request_id,
      expiresAtMs: body.expires_at_ms,
      events: [],
    });
    await refreshCatalog();
  };

  const cancelAuthentication = async (): Promise<void> => {
    try {
      if (flow?.requestId) {
        await fetch(
          `/api/providers/${
            encodeURIComponent(flow.provider.provider_id)
          }/auth/${encodeURIComponent(flow.requestId)}`,
          { method: "DELETE" },
        );
      }
    } finally {
      setLoginInput("");
      setFlow(null);
      await refreshCatalog();
    }
  };

  const submitAuthentication = async (): Promise<void> => {
    if (!flow?.requestId || !loginInput.trim()) return;
    const copy = authenticationCopy(
      resolveProviderAuthenticationPresentation(
        flow.provider.manifest.authentication,
      ),
    );
    const response = await fetch(
      `/api/providers/${encodeURIComponent(flow.provider.provider_id)}/auth/${
        encodeURIComponent(flow.requestId)
      }`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ code: loginInput.trim() }),
      },
    );
    await expectSuccess(response, copy.submitFailed);
    setLoginInput("");
  };

  const confirmUninstall = async (): Promise<void> => {
    if (!uninstallPlan) return;
    const response = await fetch(
      `/api/machines/${
        encodeURIComponent(uninstallPlan.machine_id)
      }/providers/${encodeURIComponent(uninstallPlan.provider_id)}/uninstall`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          plan_id: uninstallPlan.plan_id,
          confirm_active_sessions: confirmActive,
        }),
      },
    );
    await expectSuccess(response, "Provider uninstall failed");
    setUninstallPlan(null);
    setConfirmActive(false);
    await refreshInventory();
  };

  const challenge = flow?.events.findLast((event) =>
    event.event === "login_challenge"
  );
  const loginState = flow?.events.findLast((event) =>
    event.event === "login_state"
  );

  return (
    <Stack spacing={1.25}>
      <Stack
        direction="row"
        spacing={1}
        alignItems="center"
        justifyContent="space-between"
      >
        <Box>
          <Typography variant="overline" color="text.secondary">
            {scope === "service"
              ? "Cowboy Service authentication"
              : "Providers"}
          </Typography>
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ display: "block" }}
          >
            {scope === "service"
              ? "Configure once; encrypted credentials synchronize to every enrolled Machine"
              : "Installed, upgraded, and uninstalled independently on this Machine"}
          </Typography>
        </Box>
        {catalogError
          ? <Chip size="small" color="error" label="Catalog unavailable" />
          : null}
      </Stack>
      {catalogError ? <Alert severity="error">{catalogError}</Alert> : null}
      {!catalog && !catalogError
        ? <Typography variant="caption">Loading Provider Catalog…</Typography>
        : null}
      {catalog
        ? (
          <Stack
            direction="row"
            spacing={0.75}
            alignItems="center"
            flexWrap="wrap"
            useFlexGap
          >
            {providerRows.map((row) => {
              const entry = row.installedEntry ?? row.latestEntry;
              if (!entry) {
                return (
                  <Chip
                    key={row.providerId}
                    size="small"
                    variant="outlined"
                    color="warning"
                    label={`${row.providerId} · package unavailable`}
                  />
                );
              }
              const auth = authenticationForEntry(entry);
              const authPresentation =
                resolveProviderAuthenticationPresentation(
                  entry.manifest.authentication,
                );
              const presentationEntry = scope === "machine" && row.installed
                ? providerPresentationEntry(
                  catalog.providers,
                  row.installed.provider_id,
                  row.installed.provider_version,
                  row.installed.generation_digest,
                ) ?? entry
                : entry;
              const summary = scope === "service"
                ? entry.manifest.authentication.required
                ? serviceAuthenticationLabel(
                  auth,
                  authPresentation,
                  latestEntries.filter((candidate) =>
                    candidate.authentication_scope === entry.authentication_scope
                  ).length > 1,
                )
                  : "no sign-in"
                : providerInstallationSummary(
                  row.installed,
                  row.latestCompatibleEntry,
                  row.latestEntry,
                  row.latestCompatibility?.detail,
                );
              const healthy = scope === "service"
                ? !entry.manifest.authentication.required ||
                  auth?.authentication_state === "ready"
                : row.installed?.state === "active";
              return (
                <Chip
                  key={entry.provider_id}
                  size="small"
                  variant="outlined"
                  color={healthy ? "success" : "default"}
                  icon={
                    <ProviderMark
                      manifest={presentationEntry.manifest}
                      size={17}
                    />
                  }
                  label={`${presentationEntry.manifest.display.name} · ${summary}`}
                  sx={{
                    "& .MuiChip-icon": { ml: 0.625, mr: 0.125 },
                    "& .MuiChip-label": { pl: 0.625, pr: 0.625 },
                  }}
                />
              );
            })}
            <Button
              size="small"
              variant={detailsOpen ? "outlined" : "text"}
              aria-expanded={detailsOpen}
              aria-controls={detailsId}
              onClick={() => setDetailsOpen((current) => !current)}
              sx={{ ml: "auto" }}
            >
              {detailsOpen
                ? "Hide"
                : scope === "service"
                ? "Manage credentials"
                : "Manage"}
            </Button>
          </Stack>
        )
        : null}
      <Box
        id={detailsId}
        hidden={!detailsOpen}
        sx={{
          display: detailsOpen ? "grid" : "none",
          gridTemplateColumns: { xs: "1fr", md: "repeat(2, minmax(0, 1fr))" },
          gap: 1,
        }}
      >
        {providerRows.map((row) => {
          const {
            providerId,
            latestEntry,
            latestCompatibleEntry,
            latestCompatibility,
            installed,
            installedEntry,
          } = row;
          if (scope === "machine" && installed && !installedEntry) {
            const operationError = errors[providerId];
            const canUpgrade =
              latestCompatibleEntry?.release_state === "ready" &&
              latestCompatibleEntry.artifact_digest !== null &&
              latestCompatibleEntry.artifact_digest !==
                installed.generation_digest;
            return (
              <Paper
                key={`${providerId}:${installed.provider_version}:${installed.generation_digest}`}
                variant="outlined"
                sx={{ p: 1.25, minWidth: 0, borderRadius: 2 }}
              >
                <Stack spacing={1.1}>
                  <Typography variant="subtitle1" fontWeight={700}>
                    {providerId}
                  </Typography>
                  <Stack
                    direction="row"
                    spacing={0.75}
                    flexWrap="wrap"
                    useFlexGap
                  >
                    <Chip
                      size="small"
                      variant="outlined"
                      color="warning"
                      label={`Machine ${installed.provider_version}`}
                    />
                    <Chip
                      size="small"
                      variant="outlined"
                      label={installed.state}
                    />
                  </Stack>
                  <Alert severity="warning">
                    {operationError || latestCompatibility?.detail ||
                      "The exact installed Provider package is missing from the Service Catalog. Cowboy will not render a different release's UI."}
                  </Alert>
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    sx={{ overflowWrap: "anywhere" }}
                  >
                    Installed generation: {installed.generation_digest}
                  </Typography>
                  <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
                    {canUpgrade && latestCompatibleEntry
                      ? (
                        <Button
                          variant="contained"
                          onClick={() =>
                            void run(latestCompatibleEntry, installed, {
                              capability: "upgrade_on_machine",
                            }).catch(() => undefined)}
                        >
                          Upgrade to trusted {latestCompatibleEntry.provider_version}
                        </Button>
                      )
                      : null}
                    <Button
                      color="error"
                      variant="outlined"
                      onClick={() =>
                        void requestUninstallPlan(providerId).catch(
                          (cause: unknown) => {
                            setErrors((current) => ({
                              ...current,
                              [providerId]: cause instanceof Error
                                ? cause.message
                                : "Could not prepare Provider uninstall",
                            }));
                          },
                        )}
                    >
                      Uninstall…
                    </Button>
                  </Stack>
                </Stack>
              </Paper>
            );
          }
          if (!latestEntry) return null;
          const operationEntry = latestCompatibleEntry ?? latestEntry;
          const entry = installedEntry ?? operationEntry;
          const auth = authenticationForEntry(entry);
          const authPresentation = resolveProviderAuthenticationPresentation(
            entry.manifest.authentication,
          );
          const error = errors[entry.provider_id] || "";
          const host = scope === "service"
            ? providerServiceHost(entry, auth, error)
            : providerHost(
              entry,
              latestCompatibleEntry,
              installed,
              auth,
              machine,
              error,
              latestCompatibility?.detail ?? "",
            );
          const releaseReady = operationEntry.release_state === "ready" &&
            operationEntry.artifact_digest !== null &&
            (scope === "service" || latestCompatibleEntry !== undefined);
          const managementStatus = scope === "service"
            ? entry.manifest.authentication.required
              ? serviceAuthenticationLabel(
                auth,
                authPresentation,
                latestEntries.filter((candidate) =>
                  candidate.authentication_scope === entry.authentication_scope
                ).length > 1,
              )
                .split(" · ")[0] ?? authenticationCopy(authPresentation).empty
              : "no sign-in"
            : providerInstallationSummary(
              installed,
              latestCompatibleEntry,
              latestEntry,
              latestCompatibility?.detail,
            );
          const managementStatusTone: ProviderManagementStatusTone =
            scope === "service"
              ? !entry.manifest.authentication.required ||
                  auth?.authentication_state === "ready"
                ? "success"
                : "warning"
              : managementStatus === "active"
              ? "success"
              : managementStatus === "update available"
              ? "warning"
              : "default";
          const blockedCapabilities: ReadonlySet<EffectCapability> | undefined =
            releaseReady
            ? undefined
            : scope === "service"
            ? UNPUBLISHED_AUTH_EFFECTS
            : UNPUBLISHED_RELEASE_EFFECTS;
          const lifecycleSurface = scope === "service"
            ? entry.manifest.authentication.required
              ? (
                <ProviderManagementLifecycleSurface
                  manifest={entry.manifest}
                  slot="setup"
                  host={host}
                  blockedCapabilities={blockedCapabilities}
                  onEffect={(effect) => run(operationEntry, undefined, effect)}
                />
              )
              : null
            : !installed
            ? (
              <ProviderManagementLifecycleSurface
                manifest={entry.manifest}
                slot="empty"
                host={host}
                blockedCapabilities={blockedCapabilities}
                onEffect={(effect) => run(operationEntry, installed, effect)}
              />
            )
            : (
              <ProviderManagementLifecycleSurface
                manifest={entry.manifest}
                slot="settings"
                host={host}
                blockedCapabilities={blockedCapabilities}
                onEffect={(effect) => run(operationEntry, installed, effect)}
              />
            );
          return (
            <Paper
              key={`${entry.provider_id}:${entry.provider_version}:${
                entry.artifact_digest ?? entry.package_digest
              }`}
              variant="outlined"
              sx={{
                p: 1.25,
                minWidth: 0,
                borderRadius: 2,
              }}
              data-provider-management-card
            >
              <Stack spacing={1.25}>
                <ProviderManagementIdentity
                  manifest={entry.manifest}
                  version={host.provider_version}
                  statusLabel={managementStatus}
                  statusTone={managementStatusTone}
                  actions={lifecycleSurface}
                />
                {scope === "machine" && installed &&
                    entry.manifest.authentication.required
                  ? (
                    <Box sx={{ pl: 5 }}>
                      <Chip
                        size="small"
                        variant="outlined"
                        label={`Credentials ${
                          installed.materialization_state.replaceAll("_", " ")
                        }`}
                        color={installed.materialization_state === "current"
                          ? "success"
                          : "warning"}
                      />
                    </Box>
                  )
                  : null}
                {scope === "machine" && latestCompatibility &&
                    (!latestCompatibleEntry ||
                      latestCompatibleEntry.artifact_digest ===
                        installed?.generation_digest)
                  ? (
                    <Alert severity="warning">
                      {latestCompatibility.detail}
                    </Alert>
                  )
                  : null}
                {error || !releaseReady
                  ? (
                    <ProviderSurface
                      manifest={entry.manifest}
                      slot="error"
                      host={host}
                    />
                  )
                  : null}
              </Stack>
            </Paper>
          );
        })}
      </Box>

      <Dialog
        open={flow !== null}
        onClose={() => void cancelAuthentication()}
        fullWidth
        maxWidth="sm"
      >
        <DialogTitle sx={{ display: "flex", alignItems: "center", gap: 1 }}>
          {flow ? <ProviderMark manifest={flow.provider.manifest} /> : null}
          {flow
            ? authenticationCopy(
              resolveProviderAuthenticationPresentation(
                flow.provider.manifest.authentication,
              ),
              flow.sharedProviderNames.length > 1,
            ).title
            : "Configure"} {flow?.provider.manifest.display.name ?? "Provider"}
        </DialogTitle>
        <DialogContent>
          {flow
            ? (
              <Stack spacing={1.5} sx={{ pt: 0.5 }}>
                <Alert severity="info">
                  {authenticationCopy(
                    resolveProviderAuthenticationPresentation(
                      flow.provider.manifest.authentication,
                    ),
                    flow.sharedProviderNames.length > 1,
                  ).serviceDetail}
                </Alert>
                {!flow.requestId
                  ? (
                    <Stack spacing={1}>
                      <Typography variant="body2">
                        {authenticationCopy(
                          resolveProviderAuthenticationPresentation(
                            flow.provider.manifest.authentication,
                          ),
                          flow.sharedProviderNames.length > 1,
                        ).chooseMethod}
                      </Typography>
                      {flow.provider.manifest.authentication.methods.map((
                        method,
                      ) => (
                        <Button
                          key={method.id}
                          variant="contained"
                          onClick={() => void startAuthentication(method.id)}
                        >
                          {method.label}
                        </Button>
                      ))}
                    </Stack>
                  )
                  : null}
                {challenge?.event === "login_challenge"
                  ? (
                    <Stack spacing={1}>
                      <Button
                        component={Link}
                        href={challenge.verification_url}
                        target="_blank"
                        rel="noreferrer"
                        variant="contained"
                      >
                        {authenticationCopy(
                          resolveProviderAuthenticationPresentation(
                            flow.provider.manifest.authentication,
                          ),
                        ).externalAction}
                      </Button>
                      {challenge.user_code
                        ? (
                          <Button
                            variant="outlined"
                            onClick={() =>
                              void navigator.clipboard.writeText(
                                challenge.user_code ?? "",
                              )}
                          >
                            Copy {challenge.user_code}
                          </Button>
                        )
                        : null}
                      {challenge.input_required
                        ? (
                          <Stack spacing={1}>
                            <TextField
                              label={challenge.input_label ??
                                "Authorization value"}
                              type={challenge.secret_input
                                ? "password"
                                : "text"}
                              value={loginInput}
                              autoComplete="off"
                              onChange={(event) =>
                                setLoginInput(event.target.value)}
                            />
                            <Button
                              variant="contained"
                              disabled={!loginInput.trim()}
                              onClick={() => void submitAuthentication()}
                            >
                              {authenticationCopy(
                                resolveProviderAuthenticationPresentation(
                                  flow.provider.manifest.authentication,
                                ),
                              ).submit}
                            </Button>
                          </Stack>
                        )
                        : null}
                    </Stack>
                  )
                  : flow.requestId
                  ? (
                    <Typography variant="body2">
                      {authenticationCopy(
                        resolveProviderAuthenticationPresentation(
                          flow.provider.manifest.authentication,
                        ),
                      ).waiting}
                    </Typography>
                  )
                  : null}
                {loginState?.event === "login_state"
                  ? (
                    <Alert
                      severity={loginState.state === "signed_in" ||
                          loginState.state === "ready"
                        ? "success"
                        : "info"}
                    >
                      {loginState.detail ??
                        loginState.state.replaceAll("_", " ")}
                    </Alert>
                  )
                  : null}
              </Stack>
            )
            : null}
        </DialogContent>
        <DialogActions>
          <Button color="inherit" onClick={() => void cancelAuthentication()}>
            {loginState?.event === "login_state" &&
                (loginState.state === "signed_in" ||
                  loginState.state === "ready")
              ? "Done"
              : "Cancel"}
          </Button>
        </DialogActions>
      </Dialog>

      <Dialog
        open={uninstallPlan !== null}
        onClose={() => setUninstallPlan(null)}
        fullWidth
        maxWidth="sm"
      >
        <DialogTitle>
          Uninstall{" "}
          {latestEntries.find((entry) =>
            entry.provider_id === uninstallPlan?.provider_id
          )?.manifest.display.name ?? "Provider"} from{" "}
          {machine?.display_name ?? "Machine"}?
        </DialogTitle>
        <DialogContent>
          {uninstallPlan
            ? (
              <Stack spacing={1.5} sx={{ pt: 0.5 }}>
                <Alert severity="warning">{uninstallPlan.warning}</Alert>
                <Typography variant="body2">
                  {uninstallPlan.affected_sessions.length -
                    uninstallPlan.active_session_ids.length} idle and{" "}
                  {uninstallPlan.active_session_ids.length}{" "}
                  active session{uninstallPlan.affected_sessions.length === 1
                    ? ""
                    : "s"}{" "}
                  will leave ordinary Cowboy views immediately. Active turns are
                  explicitly cancelled; they are not silently drained.
                </Typography>
                {uninstallPlan.affected_sessions.length > 0
                  ? (
                    <Paper
                      variant="outlined"
                      sx={{ p: 1, maxHeight: 180, overflow: "auto" }}
                    >
                      <Stack spacing={0.5}>
                        {uninstallPlan.affected_sessions.map((session) => (
                          <Typography key={session.id} variant="caption">
                            {session.title} · {session.status} · {session.id}
                          </Typography>
                        ))}
                      </Stack>
                    </Paper>
                  )
                  : null}
                <Typography variant="body2" fontWeight={700}>
                  Permanent purge deadline:{" "}
                  {absolutePurgeTime(uninstallPlan.purge_after_ms)}
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  Until then Cowboy retains the affected transcripts, queued
                  messages and drafts, attachments, session metadata, and exact
                  runtime identifiers as soft-deleted data. Reinstalling does
                  not automatically restore them; the deadline is absolute and
                  is not extended by retries.
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  Source projects, repositories, session worktrees, and files
                  outside Cowboy's session database are not deleted. Cowboy
                  Service authentication remains signed in, while this Machine's
                  private credential projection is wiped.
                </Typography>
                {uninstallPlan.active_session_ids.length > 0
                  ? (
                    <FormControlLabel
                      control={
                        <Checkbox
                          checked={confirmActive}
                          onChange={(event) =>
                            setConfirmActive(event.target.checked)}
                        />
                      }
                      label={`Stop and remove ${uninstallPlan.active_session_ids.length} active session${
                        uninstallPlan.active_session_ids.length === 1 ? "" : "s"
                      }`}
                    />
                  )
                  : null}
              </Stack>
            )
            : null}
        </DialogContent>
        <DialogActions>
          <Button color="inherit" onClick={() => setUninstallPlan(null)}>
            Cancel
          </Button>
          <Button
            color="error"
            variant="contained"
            disabled={Boolean(uninstallPlan?.active_session_ids.length) &&
              !confirmActive}
            onClick={() =>
              void confirmUninstall().catch((cause: unknown) => {
                const detail = cause instanceof Error
                  ? cause.message
                  : "Provider uninstall failed";
                if (uninstallPlan) {
                  setErrors((current) => ({
                    ...current,
                    [uninstallPlan.provider_id]: detail,
                  }));
                }
              })}
          >
            Uninstall and remove sessions
          </Button>
        </DialogActions>
      </Dialog>
    </Stack>
  );
}

function providerServiceHost(
  entry: ProviderCatalogEntry,
  auth: ProviderAuthenticationStatus | undefined,
  error: string,
): ProviderHostContext {
  return {
    provider_version: entry.provider_version,
    installation_state: "Cowboy Service",
    authentication_state: auth?.authentication_state ?? "signed out",
    distribution_state: auth?.distribution_state ?? "none",
    machine_name: "",
    error_detail: error || entry.release_detail || "",
    installed: false,
    auth_ready: !entry.manifest.authentication.required ||
      auth?.authentication_state === "ready",
    auth_required: entry.manifest.authentication.required,
    machine_online: false,
    upgrade_available: false,
  };
}

function serviceAuthenticationLabel(
  auth: ProviderAuthenticationStatus | undefined,
  presentation: ProviderAuthenticationPresentation,
  shared = false,
): string {
  const copy = authenticationCopy(presentation, shared);
  if (!auth) return copy.empty;
  const state = auth.authentication_state.replaceAll("_", " ");
  if (auth.authentication_state !== "ready") {
    if (presentation === "api_key") {
      return auth.authentication_state === "authenticating"
        ? "Saving API key"
        : auth.authentication_state === "expired"
        ? "API key expired"
        : auth.authentication_state === "error"
        ? "API key error"
        : copy.empty;
    }
    return state;
  }
  return `${shared ? "shared API key configured" : copy.ready}${auth.account_label ? ` · ${auth.account_label}` : ""}`;
}

function authenticationCopy(
  presentation: ProviderAuthenticationPresentation,
  shared = false,
): {
  title: string;
  empty: string;
  ready: string;
  serviceDetail: string;
  chooseMethod: string;
  externalAction: string;
  submit: string;
  submitFailed: string;
  clearFailed: string;
  waiting: string;
} {
  switch (presentation) {
    case "account":
      return {
        title: "Sign in to",
        empty: "signed out",
        ready: "signed in",
        serviceDetail:
          "This sign-in belongs to Cowboy Service. A temporary executor performs the Provider flow; the resulting encrypted generation synchronizes to every enrolled Machine.",
        chooseMethod: "Choose a Provider-declared sign-in method.",
        externalAction: "Open sign-in page",
        submit: "Continue",
        submitFailed: "Could not submit the authentication value",
        clearFailed: "Provider sign-out failed",
        waiting: "Waiting for the Provider…",
      };
    case "api_key":
      {
        const apiKeyCopy = {
          title: "Configure API key for",
          empty: "API key missing",
          ready: "API key configured",
          serviceDetail:
            "This API key belongs to Cowboy Service. Cowboy stores one encrypted credential generation and synchronizes it to every enrolled Machine.",
          chooseMethod: "Choose the Provider-declared API key credential.",
          externalAction: "Get API key",
          submit: "Save API key",
          submitFailed: "Could not save the API key",
          clearFailed: "Could not clear the API key",
          waiting: "Preparing secure API key entry…",
        };
        if (!shared) return apiKeyCopy;
        return {
          ...apiKeyCopy,
          title: "Configure shared API key for",
          empty: "Shared API key missing",
          serviceDetail:
            "One DeepSeek API key belongs to Cowboy Service and is shared by every compatible DeepSeek Provider. Cowboy stores one encrypted credential generation and synchronizes each Provider projection to every enrolled Machine.",
          chooseMethod:
            "Enter the shared DeepSeek API key once. It will configure every compatible Provider.",
        };
      }
    default:
      return assertUnhandled(presentation);
  }
}

function providerInstallationSummary(
  installed: MachineProviderInventory | undefined,
  latestCompatible: ProviderCatalogEntry | undefined,
  latest: ProviderCatalogEntry | undefined,
  compatibilityDetail?: string,
): string {
  if (!installed) {
    return !latestCompatible && compatibilityDetail
      ? "Machine update required"
      : "not installed";
  }
  if (
    latestCompatible?.release_state === "ready" &&
    latestCompatible.artifact_digest !== null &&
    latestCompatible.artifact_digest !== installed.generation_digest
  ) return "update available";
  if (
    compatibilityDetail && latest?.release_state === "ready" &&
    latest.artifact_digest !== null &&
    latest.artifact_digest !== installed.generation_digest
  ) return "Machine update required";
  return installed.state.replaceAll("_", " ");
}

function providerHost(
  entry: ProviderCatalogEntry,
  latestCompatibleEntry: ProviderCatalogEntry | undefined,
  installed: MachineProviderInventory | undefined,
  auth: ProviderAuthenticationStatus | undefined,
  machine: ProviderMachine,
  error: string,
  compatibilityDetail: string,
): ProviderHostContext {
  return {
    provider_version: installed?.provider_version ?? entry.provider_version,
    installation_state: installed?.state ?? "not installed",
    authentication_state: auth?.authentication_state ?? "signed out",
    distribution_state: auth?.distribution_state ?? "none",
    machine_name: machine.display_name,
    error_detail: error || compatibilityDetail ||
      latestCompatibleEntry?.release_detail || "",
    installed: installed !== undefined,
    auth_ready: !entry.manifest.authentication.required ||
      auth?.authentication_state === "ready",
    auth_required: entry.manifest.authentication.required,
    machine_online: machine.status === "online" || machine.schedulable,
    upgrade_available: latestCompatibleEntry?.release_state === "ready" &&
      latestCompatibleEntry.artifact_digest !== null &&
      installed !== undefined &&
      installed.generation_digest !== latestCompatibleEntry.artifact_digest,
  };
}

async function expectSuccess(
  response: Response,
  fallback: string,
): Promise<void> {
  if (response.ok) return;
  const body = (await response.text()).trim();
  let parsed: { detail?: unknown; error?: unknown } | undefined;
  try {
    parsed = JSON.parse(body) as { detail?: unknown; error?: unknown };
  } catch { /* Preserve a non-JSON server response below. */ }
  if (typeof parsed?.detail === "string" && parsed.detail.trim()) {
    throw new Error(parsed.detail.trim());
  }
  if (typeof parsed?.error === "string" && parsed.error.trim()) {
    throw new Error(parsed.error.trim());
  }
  throw new Error(body || fallback);
}

function assertUnhandled(value: never): never {
  throw new Error(`Unhandled Provider effect ${String(value)}`);
}

function absolutePurgeTime(epochMs: number): string {
  const date = new Date(epochMs);
  return `${
    new Intl.DateTimeFormat(undefined, {
      dateStyle: "full",
      timeStyle: "long",
    }).format(date)
  } · ${date.toISOString()}`;
}
