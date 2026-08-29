export type AdminRole = "owner" | "operator" | "viewer";
export type RegistrationMode = "disabled" | "token" | "open";

export interface AdminOverview {
  healthy: boolean;
  persistence: string;
  backend: string;
  sessions_live: number;
  sessions_deleted: number;
  events_rows: number;
  daemon_rss_bytes: number;
  runtime_workers: number;
  runtime_busy_workers: number;
  registration: RegistrationPolicy;
}

export interface AdminSession {
  id: string;
  title: string;
  provider: string;
  machine_id: string;
  status: string;
}

export interface AdminMachine {
  id: string;
  display_name: string;
  connection_mode: string;
  status: string;
  last_seen_at_ms: number | null;
}

export interface RegistrationToken {
  id: string;
  name: string;
  token_prefix: string;
  uses_allowed: number | null;
  uses_count: number;
  expires_at_ms: number | null;
  created_at_ms: number;
  disabled: boolean;
}

export interface RegistrationPolicy {
  enabled: boolean;
  mode: RegistrationMode;
  accepts_registration: boolean;
  tokens: RegistrationToken[];
}

export interface PermissionPolicy {
  default_role: AdminRole;
  grants: { account: string; role: AdminRole }[];
}

export interface SessionLimits {
  max_sessions: number | null;
  max_retention_days: number | null;
  last_n: number | null;
  last_time_hours: number | null;
}

export interface AdminAuthStatus {
  authenticated: boolean;
  bootstrap_required: boolean;
  setup_pending?: boolean;
  account?: string;
  role?: AdminRole;
  passkey_count?: number;
  passkey_reauth_enabled?: boolean;
  passkey_reauth_required?: boolean;
}

export interface AdminPasskey {
  id: string;
  nickname: string;
  created_at_ms: number;
  last_used_at_ms?: number | null;
}

export interface AdminUser {
  account: string;
  role: AdminRole;
  created_at_ms: number;
}

export interface ProductUser {
  id: string;
  username: string;
  created_at_ms: number;
  updated_at_ms: number;
  disabled_at_ms: number | null;
  role: AdminRole;
}

export interface PluginRelease {
  plugin_id: string;
  plugin_version: string;
  plugin_kind:
    | "agent_provider"
    | "authentication_provider"
    | "code_intelligence";
  package_digest: string;
  artifact_digest: string | null;
  release_state: string;
  release_detail?: string;
  publisher: string;
}

async function readJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    cache: "no-store",
    credentials: "same-origin",
    headers: { accept: "application/json", ...init?.headers },
    ...init,
  });
  const text = await response.text();
  if (!response.ok) throw new Error(text || response.statusText);
  return (text ? JSON.parse(text) : {}) as T;
}

export const adminApi = {
  auth: () => readJson<AdminAuthStatus>("/api/admin/auth"),
  setup: (token: string) =>
    readJson<AdminAuthStatus>("/api/admin/auth/setup", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ token }),
    }),
  bootstrap: (account: string, password: string) =>
    readJson<AdminAuthStatus>("/api/admin/auth/bootstrap", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ account, password }),
    }),
  login: (account: string, password: string) =>
    readJson<AdminAuthStatus>("/api/admin/auth/login", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ account, password }),
    }),
  logout: () =>
    readJson<AdminAuthStatus>("/api/admin/auth/logout", { method: "POST" }),
  accounts: () => readJson<{ accounts: AdminUser[] }>("/api/admin/accounts"),
  createAccount: (account: string, password: string) =>
    readJson<{ account: string; role: AdminRole }>("/api/admin/accounts", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ account, password }),
    }),
  productUsers: () => readJson<{ users: ProductUser[] }>("/api/admin/users"),
  createProductUser: (
    account: string,
    password: string,
    role: AdminRole = "operator",
  ) =>
    readJson<ProductUser>("/api/admin/users", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ account, password, role }),
    }),
  disableProductUser: (id: string) =>
    readJson<ProductUser>(`/api/admin/users/${id}/disable`, { method: "POST" }),
  setProductUserPassword: (id: string, password: string) =>
    readJson<Record<string, never>>(`/api/admin/users/${id}/password`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ password }),
    }),
  overview: () => readJson<AdminOverview>("/api/admin/overview"),
  sessions: () => readJson<{ sessions: AdminSession[] }>("/api/admin/sessions"),
  machines: () => readJson<{ machines: AdminMachine[] }>("/api/admin/machines"),
  registration: () => readJson<RegistrationPolicy>("/api/admin/registration"),
  saveRegistration: (enabled: boolean, mode: RegistrationMode) =>
    readJson<RegistrationPolicy>("/api/admin/registration", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ enabled, mode }),
    }),
  issueToken: (
    name: string,
    usesAllowed: number | null,
    ttlSeconds: number | null,
  ) =>
    readJson<{ token: string; record: RegistrationToken }>(
      "/api/admin/registration/tokens",
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          name,
          uses_allowed: usesAllowed,
          ttl_seconds: ttlSeconds,
        }),
      },
    ),
  disableToken: (id: string) =>
    readJson<RegistrationPolicy>(`/api/admin/registration/tokens/${id}`, {
      method: "DELETE",
    }),
  permissions: () => readJson<PermissionPolicy>("/api/admin/permissions"),
  savePermissions: (policy: PermissionPolicy) =>
    readJson<PermissionPolicy>("/api/admin/permissions", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(policy),
    }),
  sessionLimits: () => readJson<SessionLimits>("/api/admin/session-limits"),
  saveSessionLimits: (limits: SessionLimits) =>
    readJson<SessionLimits>("/api/admin/session-limits", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(limits),
    }),
  plugins: () =>
    readJson<{ plugins: PluginRelease[]; catalog_root: string | null }>(
      "/api/admin/plugins",
    ),
  refreshCatalog: () =>
    readJson<{ external_releases: number }>("/api/admin/plugins/refresh", {
      method: "POST",
    }),
  listPasskeys: () =>
    readJson<{ passkeys: AdminPasskey[]; reauth_after_ms: number }>(
      "/api/admin/passkeys",
    ),
  startPasskeyRegister: (nickname: string) =>
    readJson<{ challenge_id: string; publicKey: Record<string, unknown> }>(
      "/api/admin/passkeys/register/options",
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ nickname }),
      },
    ),
  completePasskeyRegister: (
    challengeId: string,
    credential: Record<string, unknown>,
  ) =>
    readJson<AdminPasskey>("/api/admin/passkeys/register/complete", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ challenge_id: challengeId, credential }),
    }),
  startPasskeyAssert: () =>
    readJson<{ challenge_id: string; publicKey: Record<string, unknown> }>(
      "/api/admin/passkeys/assert/options",
      { method: "POST" },
    ),
  completePasskeyAssert: (
    challengeId: string,
    credential: Record<string, unknown>,
  ) =>
    readJson<AdminAuthStatus>("/api/admin/passkeys/assert/complete", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ challenge_id: challengeId, credential }),
    }),
  setPasskeyReauth: (enabled: boolean) =>
    readJson<AdminAuthStatus>("/api/admin/passkeys/reauth", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ enabled }),
    }),
  deletePasskey: (id: string) =>
    readJson<{ ok: boolean }>(`/api/admin/passkeys/${id}`, {
      method: "DELETE",
    }),
};
