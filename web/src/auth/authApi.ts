export type ProductRole = "owner" | "operator" | "viewer";
export type RegistrationMode = "disabled" | "token" | "open";

export interface RegistrationPublicStatus {
  enabled: boolean;
  mode: RegistrationMode;
  accepts_registration: boolean;
}

export interface ProductMe {
  account: string;
  role: ProductRole;
  auth_enabled?: boolean;
  primary_auth_method?: string | null;
  passkey_count?: number;
  passkey_reauth_enabled?: boolean;
  passkey_reauth_required?: boolean;
  passkey_reauth_after_ms?: number;
  passkey_reauth_due_at_ms?: number | null;
  passkey_reauth_warn_at_ms?: number | null;
  primary_reauth_due_at_ms?: number | null;
  primary_reauth_warn_at_ms?: number | null;
  session_idle_due_at_ms?: number | null;
  session_expires_at_ms?: number | null;
  session_server_now_ms?: number | null;
  session_reauth_kind?: "passkey" | "primary" | null;
}

export interface ProductOidcProvider {
  id: string;
  display_name: string;
  button_label: string;
  start_url: string;
}

export const PASSWORD_LOGIN_METHOD = "password";

export function defaultProductLoginMethodOrder(
  passwordEnabled: boolean,
  providers: ProductOidcProvider[],
): string[] {
  const providerIds = [...new Set(providers.map((provider) => provider.id))];
  const order: string[] = [];
  if (providerIds.includes("cardea")) order.push("cardea");
  if (passwordEnabled) order.push(PASSWORD_LOGIN_METHOD);
  order.push(...providerIds.filter((id) => id !== "cardea"));
  return order;
}

export function resolveProductLoginMethodOrder(
  configured: unknown,
  passwordEnabled: boolean,
  providers: ProductOidcProvider[],
): string[] {
  const fallback = defaultProductLoginMethodOrder(passwordEnabled, providers);
  if (!Array.isArray(configured) || configured.length !== fallback.length) {
    return fallback;
  }
  if (
    !configured.every((method): method is string => typeof method === "string")
  ) {
    return fallback;
  }
  const configuredSet = new Set(configured);
  const availableSet = new Set(fallback);
  if (
    configuredSet.size !== configured.length ||
    configuredSet.size !== availableSet.size ||
    ![...configuredSet].every((method) => availableSet.has(method))
  ) {
    return fallback;
  }
  return [...configured];
}

export type NativeOidcPoll = { status: "pending" } | ProductMe;

export function nativeOidcPollPath(provider: ProductOidcProvider): string {
  return provider.start_url === "/api/auth/oidc/start"
    ? "/api/auth/oidc/native/poll"
    : `/api/auth/providers/${encodeURIComponent(provider.id)}/native/poll`;
}

export function nativeOidcEventsPath(provider: ProductOidcProvider): string {
  return provider.start_url === "/api/auth/oidc/start"
    ? "/api/auth/oidc/native/events"
    : `/api/auth/providers/${encodeURIComponent(provider.id)}/native/events`;
}

export function nativeOidcCancelPath(provider: ProductOidcProvider): string {
  return provider.start_url === "/api/auth/oidc/start"
    ? "/api/auth/oidc/native/cancel"
    : `/api/auth/providers/${encodeURIComponent(provider.id)}/native/cancel`;
}

export interface ProductPasskeyServerPolicy {
  enabled: boolean;
  prompt_after_login: boolean;
  session_refresh_enabled: boolean;
}

export interface ProductSessionServerPolicy {
  activity_sliding_enabled: boolean;
  idle_timeout_ms: number;
  passkey_max_age_ms: number;
  passkey_warning_ms: number;
  primary_max_age_ms: number;
  primary_warning_ms: number;
}

export interface ProductCapacityServerPolicy {
  enforcement: "observe" | "enforce";
  authorized_clients_per_user: number;
  signed_in_sessions_per_user: number;
  active_clients_per_user: number;
  active_clients_service: number;
  websocket_channels_per_client: number;
  active_lease_ms: number;
  heartbeat_ms: number;
  reservation_ms: number;
  session_overflow: "revoke_oldest_inactive";
  active_overflow: "wait_or_reclaim_own";
  single_session_mode: "off" | "newest_wins";
}

export interface ProductLogoutServerPolicy {
  provider_logout: "never" | "offer" | "always";
  backchannel_logout: boolean;
}

export interface ProductAutomationServerPolicy {
  enabled: boolean;
  active_clients: number;
  credential_max_age_ms: number;
}

export interface AuthStatus {
  registration: RegistrationPublicStatus;
  setup_required?: boolean;
  setup_pending?: boolean;
  password_enabled?: boolean;
  login_method_order?: string[];
  passkeys?: ProductPasskeyServerPolicy;
  session?: ProductSessionServerPolicy;
  capacity?: ProductCapacityServerPolicy;
  logout?: ProductLogoutServerPolicy;
  automation?: ProductAutomationServerPolicy;
  providers?: ProductOidcProvider[];
  me?: ProductMe;
}

export function productMeFromJson(value: unknown): ProductMe | undefined {
  if (value == null || typeof value !== "object") return undefined;
  const me = value as Partial<ProductMe>;
  if (
    typeof me.account !== "string" || me.account.length === 0 ||
    (me.role !== "owner" && me.role !== "operator" && me.role !== "viewer")
  ) return undefined;
  const next: ProductMe = { account: me.account, role: me.role };
  if (typeof me.auth_enabled === "boolean") next.auth_enabled = me.auth_enabled;
  if (
    typeof me.primary_auth_method === "string" ||
    me.primary_auth_method === null
  ) {
    next.primary_auth_method = me.primary_auth_method;
  }
  if (typeof me.passkey_count === "number") {
    next.passkey_count = me.passkey_count;
  }
  if (typeof me.passkey_reauth_enabled === "boolean") {
    next.passkey_reauth_enabled = me.passkey_reauth_enabled;
  }
  if (typeof me.passkey_reauth_required === "boolean") {
    next.passkey_reauth_required = me.passkey_reauth_required;
  }
  if (typeof me.passkey_reauth_after_ms === "number") {
    next.passkey_reauth_after_ms = me.passkey_reauth_after_ms;
  }
  for (
    const key of [
      "passkey_reauth_due_at_ms",
      "passkey_reauth_warn_at_ms",
      "primary_reauth_due_at_ms",
      "primary_reauth_warn_at_ms",
      "session_idle_due_at_ms",
      "session_expires_at_ms",
      "session_server_now_ms",
    ] as const
  ) {
    const candidate = me[key];
    if (typeof candidate === "number" || candidate === null) {
      next[key] = candidate;
    }
  }
  if (
    me.session_reauth_kind === "passkey" ||
    me.session_reauth_kind === "primary" ||
    me.session_reauth_kind === null
  ) {
    next.session_reauth_kind = me.session_reauth_kind;
  }
  return next;
}

export function isHtmlContentType(contentType: string | null): boolean {
  return (contentType ?? "").toLowerCase().includes("text/html");
}

export function authStatusFromJson(value: unknown): AuthStatus | undefined {
  if (value == null || typeof value !== "object") return undefined;
  const record = value as {
    registration?: Partial<RegistrationPublicStatus>;
    setup_required?: boolean;
    setup_pending?: boolean;
    password_enabled?: boolean;
    login_method_order?: unknown;
    passkeys?: Partial<ProductPasskeyServerPolicy>;
    session?: Partial<ProductSessionServerPolicy>;
    capacity?: Partial<ProductCapacityServerPolicy>;
    logout?: Partial<ProductLogoutServerPolicy>;
    automation?: Partial<ProductAutomationServerPolicy>;
    providers?: unknown;
    me?: Partial<ProductMe>;
  };
  const registration = record.registration;
  if (
    registration == null ||
    typeof registration.enabled !== "boolean" ||
    typeof registration.accepts_registration !== "boolean" ||
    (registration.mode !== "disabled" &&
      registration.mode !== "token" &&
      registration.mode !== "open")
  ) {
    return undefined;
  }
  const status: AuthStatus = {
    registration: {
      enabled: registration.enabled,
      mode: registration.mode,
      accepts_registration: registration.accepts_registration,
    },
    setup_required: record.setup_required === true,
    setup_pending: record.setup_pending === true,
    password_enabled: record.password_enabled !== false,
    providers: [],
  };
  if (
    typeof record.session?.activity_sliding_enabled === "boolean" &&
    typeof record.session.idle_timeout_ms === "number" &&
    typeof record.session.passkey_max_age_ms === "number" &&
    typeof record.session.passkey_warning_ms === "number" &&
    typeof record.session.primary_max_age_ms === "number" &&
    typeof record.session.primary_warning_ms === "number"
  ) {
    status.session = {
      activity_sliding_enabled: record.session.activity_sliding_enabled,
      idle_timeout_ms: record.session.idle_timeout_ms,
      passkey_max_age_ms: record.session.passkey_max_age_ms,
      passkey_warning_ms: record.session.passkey_warning_ms,
      primary_max_age_ms: record.session.primary_max_age_ms,
      primary_warning_ms: record.session.primary_warning_ms,
    };
  }
  const capacity = record.capacity;
  if (
    (capacity?.enforcement === "observe" ||
      capacity?.enforcement === "enforce") &&
    typeof capacity.authorized_clients_per_user === "number" &&
    typeof capacity.signed_in_sessions_per_user === "number" &&
    typeof capacity.active_clients_per_user === "number" &&
    typeof capacity.active_clients_service === "number" &&
    typeof capacity.websocket_channels_per_client === "number" &&
    typeof capacity.active_lease_ms === "number" &&
    typeof capacity.heartbeat_ms === "number" &&
    typeof capacity.reservation_ms === "number" &&
    capacity.session_overflow === "revoke_oldest_inactive" &&
    capacity.active_overflow === "wait_or_reclaim_own" &&
    (capacity.single_session_mode === "off" ||
      capacity.single_session_mode === "newest_wins")
  ) {
    status.capacity = capacity as ProductCapacityServerPolicy;
  }
  const logout = record.logout;
  if (
    (logout?.provider_logout === "never" ||
      logout?.provider_logout === "offer" ||
      logout?.provider_logout === "always") &&
    typeof logout.backchannel_logout === "boolean"
  ) {
    status.logout = logout as ProductLogoutServerPolicy;
  }
  const automation = record.automation;
  if (
    typeof automation?.enabled === "boolean" &&
    typeof automation.active_clients === "number" &&
    typeof automation.credential_max_age_ms === "number"
  ) {
    status.automation = automation as ProductAutomationServerPolicy;
  }
  if (
    typeof record.passkeys?.enabled === "boolean" &&
    typeof record.passkeys.prompt_after_login === "boolean" &&
    typeof record.passkeys.session_refresh_enabled === "boolean"
  ) {
    status.passkeys = {
      enabled: record.passkeys.enabled,
      prompt_after_login: record.passkeys.prompt_after_login,
      session_refresh_enabled: record.passkeys.session_refresh_enabled,
    };
  }
  if (Array.isArray(record.providers)) {
    status.providers = record.providers.flatMap(
      (provider): ProductOidcProvider[] => {
        if (provider == null || typeof provider !== "object") return [];
        const candidate = provider as Partial<ProductOidcProvider>;
        return typeof candidate.id === "string" &&
            candidate.id !== PASSWORD_LOGIN_METHOD &&
            /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(candidate.id) &&
            typeof candidate.display_name === "string" &&
            candidate.display_name.length > 0 &&
            typeof candidate.button_label === "string" &&
            candidate.button_label.length > 0 &&
            (candidate.start_url ===
                `/api/auth/providers/${candidate.id}/start` ||
              candidate.id === "cardea" &&
                candidate.start_url === "/api/auth/oidc/start")
          ? [{
            id: candidate.id,
            display_name: candidate.display_name,
            button_label: candidate.button_label,
            start_url: candidate.start_url,
          }]
          : [];
      },
    );
  }
  status.login_method_order = resolveProductLoginMethodOrder(
    record.login_method_order,
    status.password_enabled !== false,
    status.providers ?? [],
  );
  const me = productMeFromJson(record.me);
  if (me) status.me = me;
  return status;
}

export class AuthApiError extends Error {
  readonly status: number;
  readonly code: string | undefined;

  constructor(message: string, status: number, code?: string) {
    super(message);
    this.name = "AuthApiError";
    this.status = status;
    this.code = code;
  }
}

export function isRecentProductAuthRequired(reason: unknown): boolean {
  return reason instanceof AuthApiError && reason.status === 428;
}

async function readJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    cache: "no-store",
    credentials: "same-origin",
    headers: { accept: "application/json", ...init?.headers },
  });
  const text = await response.text();
  if (!response.ok) {
    throw authApiError(response.status, response.statusText, text);
  }
  return (text ? JSON.parse(text) as T : {}) as T;
}

async function readPublicJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    cache: "no-store",
    credentials: "omit",
    referrerPolicy: "no-referrer",
    headers: { accept: "application/json", ...init?.headers },
  });
  const text = await response.text();
  if (!response.ok) {
    throw authApiError(response.status, response.statusText, text);
  }
  return (text ? JSON.parse(text) as T : {}) as T;
}

function authApiError(
  status: number,
  statusText: string,
  text: string,
): AuthApiError {
  if (text !== "") {
    try {
      const body = JSON.parse(text) as { message?: unknown; code?: unknown };
      if (typeof body.message === "string") {
        return new AuthApiError(
          body.message,
          status,
          typeof body.code === "string" ? body.code : undefined,
        );
      }
    } catch {
      // Preserve plain-text errors from older Cowboy services and plugins.
    }
  }
  return new AuthApiError(text || statusText, status);
}

export type AuthStatusProbe =
  | { kind: "ok"; httpStatus: 200; body: AuthStatus }
  | { kind: "unsupported"; httpStatus: number }
  | { kind: "unavailable"; httpStatus: number }
  | { kind: "network" };

export async function fetchAuthStatus(): Promise<AuthStatusProbe> {
  let response: Response;
  try {
    response = await fetch("/api/auth/status", {
      cache: "no-store",
      credentials: "same-origin",
      headers: { accept: "application/json" },
    });
  } catch {
    return { kind: "network" };
  }
  if (response.status === 404 || response.status === 501) {
    return { kind: "unsupported", httpStatus: response.status };
  }
  if (!response.ok) {
    return { kind: "unavailable", httpStatus: response.status };
  }
  if (isHtmlContentType(response.headers.get("content-type"))) {
    return { kind: "unsupported", httpStatus: response.status };
  }
  try {
    const text = await response.text();
    const body = authStatusFromJson(text ? JSON.parse(text) : {});
    if (!body) return { kind: "unsupported", httpStatus: response.status };
    return { kind: "ok", httpStatus: 200, body };
  } catch {
    return { kind: "unsupported", httpStatus: response.status };
  }
}

export interface ProductApiToken {
  id: string;
  name: string;
  token_prefix: string;
  created_at_ms: number;
  expires_at_ms?: number | null;
  last_used_at_ms?: number | null;
}

export interface CreatedProductApiToken extends ProductApiToken {
  token: string;
}

export interface ProductDevice {
  id: string;
  name: string;
  created_at_ms: number;
  last_used_at_ms?: number | null;
}

export interface ProductBrowserSession {
  id: string;
  current: boolean;
  client_kind: "browser" | "native_shell";
  principal_class: "human";
  created_at_ms: number;
  expires_at_ms: number;
  last_seen_at_ms: number;
  user_agent?: string | null;
  primary_auth_method?: string | null;
  provider_id?: string | null;
}

export interface ProductActiveClient {
  client_id: string;
  user_id?: string | null;
  principal_class: "human";
  session_id?: string | null;
  client_kind: "browser" | "native_shell" | "cli" | "acp";
  fencing_token: number;
  acquired_at_ms: number;
  heartbeat_at_ms: number;
  expires_at_ms: number;
}

export interface ProductSessionInventory {
  sessions: ProductBrowserSession[];
  active_clients: ProductActiveClient[];
  authorized_clients: number;
  limit: number;
  active_limit: number;
  enforcement: "observe" | "enforce";
}

export type ProductLogoutScope = "current" | "provider" | "all";

export interface ProductLogoutResult {
  ok: boolean;
  scope: ProductLogoutScope;
  revoked_sessions: number;
  provider_logout_url?: string | null;
}

export interface DeviceAuthorizationRequest {
  request_id: string;
  approval_token: string;
}

export interface DeviceAuthorizationInfo {
  request_id: string;
  name: string;
  fingerprint: string;
  expires_at_ms: number;
  status: "pending" | "approved" | "denied";
}

export const authApi = {
  status: () => fetchAuthStatus(),
  me: () => readJson<ProductMe>("/api/auth/me"),
  login: (account: string, password: string) =>
    readJson<ProductMe>("/api/auth/login", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ account, password }),
    }),
  pollNativeOidc: (
    provider: ProductOidcProvider,
    handoffToken: string,
    codeVerifier: string,
  ) =>
    readJson<NativeOidcPoll>(
      nativeOidcPollPath(provider),
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          handoff_token: handoffToken,
          code_verifier: codeVerifier,
        }),
      },
    ),
  cancelNativeOidc: (
    provider: ProductOidcProvider,
    handoffToken: string,
    codeVerifier: string,
  ) =>
    readJson<Record<string, never>>(
      nativeOidcCancelPath(provider),
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          handoff_token: handoffToken,
          code_verifier: codeVerifier,
        }),
      },
    ),
  setup: (token: string) =>
    readJson<AuthStatus>("/api/auth/setup", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ token }),
    }),
  register: (account: string, password: string) =>
    readJson<ProductMe>("/api/auth/register", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ account, password }),
    }),
  logout: (
    scope: ProductLogoutScope = "current",
    providerLogout = false,
  ) =>
    readJson<ProductLogoutResult>("/api/auth/logout", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ scope, provider_logout: providerLogout }),
    }),
  listSessions: () =>
    readJson<ProductSessionInventory>("/api/auth/sessions"),
  deleteSession: (id: string) =>
    readJson<{ ok: boolean }>(
      `/api/auth/sessions/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    ),
  releaseActiveClient: (clientId: string, fencingToken: number) =>
    readJson<{ ok: boolean }>(
      `/api/auth/active-clients/${encodeURIComponent(clientId)}?fencing_token=${
        encodeURIComponent(String(fencingToken))
      }`,
      { method: "DELETE" },
    ),
  listTokens: () => readJson<{ tokens: ProductApiToken[] }>("/api/auth/tokens"),
  createToken: (name: string, ttlSeconds?: number) =>
    readJson<CreatedProductApiToken>("/api/auth/tokens", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(
        ttlSeconds === undefined ? { name } : { name, ttl_seconds: ttlSeconds },
      ),
    }),
  deleteToken: (id: string) =>
    readJson<{ ok: boolean }>(`/api/auth/tokens/${id}`, { method: "DELETE" }),
  inspectDeviceAuthorization: (request: DeviceAuthorizationRequest) =>
    readPublicJson<DeviceAuthorizationInfo>(
      "/api/auth/device/authorizations/inspect",
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(request),
      },
    ),
  approveDeviceAuthorization: (request: DeviceAuthorizationRequest) =>
    readJson<{ ok: boolean }>("/api/auth/device/authorizations/approve", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(request),
    }),
  denyDeviceAuthorization: (request: DeviceAuthorizationRequest) =>
    readJson<{ ok: boolean }>("/api/auth/device/authorizations/deny", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(request),
    }),
  listDevices: () =>
    readJson<{ devices: ProductDevice[] }>("/api/auth/devices"),
  deleteDevice: (id: string) =>
    readJson<{ ok: boolean }>(`/api/auth/devices/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
  listPasskeys: () =>
    readJson<{ passkeys: ProductPasskey[]; reauth_after_ms: number }>(
      "/api/auth/passkeys",
    ),
  startPasskeyRegister: (nickname: string) =>
    readJson<PasskeyCeremony>(
      "/api/auth/passkeys/register/options",
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ nickname }),
      },
    ),
  completePasskeyRegister: (
    challengeId: string,
    credential: PublicKeyCredentialJSON,
  ) =>
    readJson<ProductPasskey>("/api/auth/passkeys/register/complete", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ challenge_id: challengeId, credential }),
    }),
  startPasskeyAssert: (signal?: AbortSignal) =>
    readJson<PasskeyCeremony>("/api/auth/passkeys/assert/options", {
      method: "POST",
      ...(signal === undefined ? {} : { signal }),
    }),
  completePasskeyAssert: (
    challengeId: string,
    credential: PublicKeyCredentialJSON,
    signal?: AbortSignal,
  ) =>
    readJson<ProductMe>("/api/auth/passkeys/assert/complete", {
      method: "POST",
      ...(signal === undefined ? {} : { signal }),
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ challenge_id: challengeId, credential }),
    }),
  startExternalPasskey: (
    action: ExternalPasskeyAction,
    codeChallenge: string,
    nickname?: string,
    signal?: AbortSignal,
  ) =>
    readJson<ExternalPasskeyStart>("/api/auth/passkeys/external/start", {
      method: "POST",
      ...(signal === undefined ? {} : { signal }),
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        action,
        code_challenge: codeChallenge,
        ...(nickname === undefined ? {} : { nickname }),
      }),
    }),
  finalizeExternalPasskey: (
    transactionId: string,
    codeVerifier: string,
    signal?: AbortSignal,
  ) =>
    readJson<ExternalPasskeyFinalize>(
      "/api/auth/passkeys/external/finalize",
      {
        method: "POST",
        ...(signal === undefined ? {} : { signal }),
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          transaction_id: transactionId,
          code_verifier: codeVerifier,
        }),
      },
    ),
  setPasskeyReauth: (enabled: boolean, reauthAfterMs?: number) =>
    readJson<ProductMe>("/api/auth/passkeys/reauth", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ enabled, reauth_after_ms: reauthAfterMs }),
    }),
  deletePasskey: (id: string) =>
    readJson<{ ok: boolean }>(`/api/auth/passkeys/${id}`, { method: "DELETE" }),
};

export const externalPasskeyApi = {
  options: (transactionId: string) =>
    readPublicJson<ExternalPasskeyBrowserState>(
      "/api/auth/passkeys/external/options",
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ transaction_id: transactionId }),
      },
    ),
  complete: (
    transactionId: string,
    credential: PublicKeyCredentialJSON,
  ) =>
    readPublicJson<{ status: "complete" }>(
      "/api/auth/passkeys/external/complete",
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ transaction_id: transactionId, credential }),
      },
    ),
  fail: (transactionId: string) =>
    readPublicJson<{ ok: boolean }>("/api/auth/passkeys/external/fail", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ transaction_id: transactionId }),
    }),
};

export interface ProductPasskey {
  id: string;
  nickname: string;
  created_at_ms: number;
  last_used_at_ms?: number | null;
}

export interface PasskeyOptions {
  publicKey:
    | PublicKeyCredentialCreationOptionsJSON
    | PublicKeyCredentialRequestOptionsJSON;
}

export interface PasskeyCeremony extends PasskeyOptions {
  challenge_id: string;
}

export type ExternalPasskeyAction = "register" | "assert";

export interface ExternalPasskeyStart {
  transaction_id: string;
  expires_in_seconds: number;
}

export type ExternalPasskeyBrowserState =
  | {
    status: "ready";
    action: ExternalPasskeyAction;
    publicKey:
      | PublicKeyCredentialCreationOptionsJSON
      | PublicKeyCredentialRequestOptionsJSON;
  }
  | { status: "complete" }
  | { status: "failed" };

export type ExternalPasskeyFinalize =
  | { status: "pending" }
  | { status: "complete"; passkey: ProductPasskey }
  | { status: "complete"; me: ProductMe };

type PublicKeyCredentialJSON = Record<string, unknown>;
type PublicKeyCredentialCreationOptionsJSON = Record<string, unknown>;
type PublicKeyCredentialRequestOptionsJSON = Record<string, unknown>;
