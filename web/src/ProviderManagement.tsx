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
import {
  type EffectCapability,
  type MachineProviderInventory,
  type ProviderAuthenticationStatus,
  type ProviderCatalogEntry,
  type ProviderHostContext,
  validateMachineProviderInventory,
} from "../../packages/provider-ui-sdk/src/index.ts";
import {
  joinProviderInstallations,
  latestProviderEntries,
  useProviderCatalog,
} from "./providerCatalog";
import { ProviderMark, ProviderSurface } from "./ProviderSurface";

interface ProviderMachine {
  id: string;
  display_name: string;
  status: string;
  schedulable: boolean;
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
  const machineId = machine?.id;
  const latestEntries = useMemo(
    () => latestProviderEntries(catalog?.providers ?? []),
    [catalog],
  );
  const providerRows = useMemo(
    () =>
      scope === "machine"
        ? joinProviderInstallations(catalog?.providers ?? [], inventory)
        : latestEntries.map((latestEntry) => ({
          providerId: latestEntry.provider_id,
          latestEntry,
          installed: undefined,
          installedEntry: undefined,
        })),
    [catalog, inventory, latestEntries, scope],
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
          setFlow({ provider: entry, events: [] });
          return;
        case "logout_service_authentication": {
          if (scope !== "service") {
            throw new Error(
              "Provider authentication is managed at Cowboy Service scope",
            );
          }
          const response = await fetch(
            `/api/providers/${encodeURIComponent(entry.provider_id)}/auth`,
            {
              method: "DELETE",
            },
          );
          await expectSuccess(response, "Provider sign-out failed");
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
    await expectSuccess(response, "Could not submit authentication value");
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
              ? "Sign in once; encrypted generations synchronize to every enrolled Machine"
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
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: { xs: "1fr", md: "repeat(2, minmax(0, 1fr))" },
          gap: 1,
        }}
      >
        {providerRows.map((row) => {
          const { providerId, latestEntry, installed, installedEntry } = row;
          if (scope === "machine" && installed && !installedEntry) {
            const operationError = errors[providerId];
            const canUpgrade = latestEntry?.release_state === "ready" &&
              latestEntry.artifact_digest !== null &&
              latestEntry.artifact_digest !== installed.generation_digest;
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
                    {operationError ||
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
                    {canUpgrade && latestEntry
                      ? (
                        <Button
                          variant="contained"
                          onClick={() =>
                            void run(latestEntry, installed, {
                              capability: "upgrade_on_machine",
                            }).catch(() => undefined)}
                        >
                          Upgrade to trusted {latestEntry.provider_version}
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
          const entry = installedEntry ?? latestEntry;
          const auth = authentications.get(entry.provider_id);
          const error = errors[entry.provider_id] || "";
          const host = scope === "service"
            ? providerServiceHost(entry, auth, error)
            : providerHost(entry, latestEntry, installed, auth, machine, error);
          const releaseReady = latestEntry.release_state === "ready" &&
            latestEntry.artifact_digest !== null;
          const blockedCapabilities = releaseReady
            ? undefined
            : scope === "service"
            ? UNPUBLISHED_AUTH_EFFECTS
            : UNPUBLISHED_RELEASE_EFFECTS;
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
                borderTop: `3px solid ${entry.manifest.display.accent}`,
              }}
            >
              <Stack spacing={1.1}>
                <ProviderSurface
                  manifest={entry.manifest}
                  slot={scope === "service" ? "information" : "card"}
                  host={host}
                />
                <Stack
                  direction="row"
                  spacing={0.75}
                  flexWrap="wrap"
                  useFlexGap
                >
                  {scope === "machine"
                    ? (
                      <Chip
                        size="small"
                        variant="outlined"
                        label={installed
                          ? `Machine ${installed.provider_version}`
                          : "Not installed"}
                        color={installed ? "success" : "default"}
                      />
                    )
                    : entry.manifest.authentication.required
                    ? (
                      <Chip
                        size="small"
                        variant="outlined"
                        label={serviceAuthenticationLabel(auth)}
                        color={auth?.authentication_state === "ready"
                          ? "success"
                          : "warning"}
                      />
                    )
                    : (
                      <Chip
                        size="small"
                        variant="outlined"
                        label="No sign-in required"
                      />
                    )}
                  {scope === "machine" && installed &&
                      entry.manifest.authentication.required
                    ? (
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
                    )
                    : null}
                </Stack>
                {error || !releaseReady
                  ? (
                    <ProviderSurface
                      manifest={entry.manifest}
                      slot="error"
                      host={host}
                    />
                  )
                  : null}
                {scope === "service"
                  ? (
                    entry.manifest.authentication.required
                      ? (
                        <ProviderSurface
                          manifest={entry.manifest}
                          slot="setup"
                          host={host}
                          blockedCapabilities={blockedCapabilities}
                          onEffect={(effect) =>
                            run(latestEntry, undefined, effect)}
                        />
                      )
                      : null
                  )
                  : !installed
                  ? (
                    <ProviderSurface
                      manifest={entry.manifest}
                      slot="empty"
                      host={host}
                      blockedCapabilities={blockedCapabilities}
                      onEffect={(effect) => run(latestEntry, installed, effect)}
                    />
                  )
                  : (
                    <ProviderSurface
                      manifest={entry.manifest}
                      slot="settings"
                      host={host}
                      blockedCapabilities={blockedCapabilities}
                      onEffect={(effect) => run(latestEntry, installed, effect)}
                    />
                  )}
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
          Sign in to {flow?.provider.manifest.display.name ?? "Provider"}
        </DialogTitle>
        <DialogContent>
          {flow
            ? (
              <Stack spacing={1.5} sx={{ pt: 0.5 }}>
                <Alert severity="info">
                  This login belongs to Cowboy Service. A temporary executor
                  performs the provider flow; the resulting encrypted generation
                  is synchronized to every enrolled Machine.
                </Alert>
                {!flow.requestId
                  ? (
                    <Stack spacing={1}>
                      <Typography variant="body2">
                        Choose a Provider-declared authentication method.
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
                        Open sign-in page
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
                              Continue
                            </Button>
                          </Stack>
                        )
                        : null}
                    </Stack>
                  )
                  : flow.requestId
                  ? (
                    <Typography variant="body2">
                      Waiting for the Provider…
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
): string {
  if (!auth) return "Service signed out";
  const state = auth.authentication_state.replaceAll("_", " ");
  if (auth.authentication_state !== "ready") return `Service ${state}`;
  return `Service signed in${
    auth.account_label ? ` · ${auth.account_label}` : ""
  }`;
}

function providerHost(
  entry: ProviderCatalogEntry,
  latestEntry: ProviderCatalogEntry,
  installed: MachineProviderInventory | undefined,
  auth: ProviderAuthenticationStatus | undefined,
  machine: ProviderMachine,
  error: string,
): ProviderHostContext {
  return {
    provider_version: installed?.provider_version ?? entry.provider_version,
    installation_state: installed?.state ?? "not installed",
    authentication_state: auth?.authentication_state ?? "signed out",
    distribution_state: auth?.distribution_state ?? "none",
    machine_name: machine.display_name,
    error_detail: error || latestEntry.release_detail || "",
    installed: installed !== undefined,
    auth_ready: !entry.manifest.authentication.required ||
      auth?.authentication_state === "ready",
    auth_required: entry.manifest.authentication.required,
    machine_online: machine.status === "online" || machine.schedulable,
    upgrade_available: latestEntry.release_state === "ready" &&
      latestEntry.artifact_digest !== null &&
      installed !== undefined &&
      installed.generation_digest !== latestEntry.artifact_digest,
  };
}

async function expectSuccess(
  response: Response,
  fallback: string,
): Promise<void> {
  if (!response.ok) throw new Error((await response.text()).trim() || fallback);
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
