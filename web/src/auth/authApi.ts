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
  passkey_count?: number;
  passkey_reauth_enabled?: boolean;
  passkey_reauth_required?: boolean;
}

export interface AuthStatus {
  registration: RegistrationPublicStatus;
  setup_required?: boolean;
  setup_pending?: boolean;
  me?: ProductMe;
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
  };
  const me = record.me;
  if (me && typeof me.account === "string" && me.account.length > 0) {
    const role = me.role;
    if (role === "owner" || role === "operator" || role === "viewer") {
      const next: ProductMe = { account: me.account, role };
      if (typeof me.auth_enabled === "boolean") next.auth_enabled = me.auth_enabled;
      if (typeof me.passkey_count === "number") next.passkey_count = me.passkey_count;
      if (typeof me.passkey_reauth_enabled === "boolean") {
        next.passkey_reauth_enabled = me.passkey_reauth_enabled;
      }
      if (typeof me.passkey_reauth_required === "boolean") {
        next.passkey_reauth_required = me.passkey_reauth_required;
      }
      status.me = next;
    }
  }
  return status;
}

export class AuthApiError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = "AuthApiError";
    this.status = status;
  }
}

async function readJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    cache: "no-store",
    credentials: "same-origin",
    headers: { accept: "application/json", ...init?.headers },
  });
  const text = await response.text();
  if (!response.ok) throw new AuthApiError(text || response.statusText, response.status);
  return (text ? JSON.parse(text) as T : {}) as T;
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

export const authApi = {
  status: () => fetchAuthStatus(),
  me: () => readJson<ProductMe>("/api/auth/me"),
  login: (account: string, password: string) =>
    readJson<ProductMe>("/api/auth/login", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ account, password }),
    }),
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
  logout: () =>
    readJson<{ ok: boolean }>("/api/auth/logout", { method: "POST" }),
  listTokens: () =>
    readJson<{ tokens: ProductApiToken[] }>("/api/auth/tokens"),
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
  completePasskeyRegister: (challengeId: string, credential: PublicKeyCredentialJSON) =>
    readJson<ProductPasskey>("/api/auth/passkeys/register/complete", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ challenge_id: challengeId, credential }),
    }),
  startPasskeyAssert: () =>
    readJson<PasskeyCeremony>("/api/auth/passkeys/assert/options", {
      method: "POST",
    }),
  completePasskeyAssert: (challengeId: string, credential: PublicKeyCredentialJSON) =>
    readJson<ProductMe>("/api/auth/passkeys/assert/complete", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ challenge_id: challengeId, credential }),
    }),
  setPasskeyReauth: (enabled: boolean) =>
    readJson<ProductMe>("/api/auth/passkeys/reauth", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ enabled }),
    }),
  deletePasskey: (id: string) =>
    readJson<{ ok: boolean }>(`/api/auth/passkeys/${id}`, { method: "DELETE" }),
};

export interface ProductPasskey {
  id: string;
  nickname: string;
  created_at_ms: number;
  last_used_at_ms?: number | null;
}

export interface PasskeyCeremony {
  challenge_id: string;
  publicKey: PublicKeyCredentialCreationOptionsJSON | PublicKeyCredentialRequestOptionsJSON;
}

type PublicKeyCredentialJSON = Record<string, unknown>;
type PublicKeyCredentialCreationOptionsJSON = Record<string, unknown>;
type PublicKeyCredentialRequestOptionsJSON = Record<string, unknown>;
