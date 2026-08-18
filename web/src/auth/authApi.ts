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
}

export interface AuthStatus {
  registration: RegistrationPublicStatus;
  me?: ProductMe;
}

export function isHtmlContentType(contentType: string | null): boolean {
  return (contentType ?? "").toLowerCase().includes("text/html");
}

export function authStatusFromJson(value: unknown): AuthStatus | undefined {
  if (value == null || typeof value !== "object") return undefined;
  const record = value as {
    registration?: Partial<RegistrationPublicStatus>;
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
  };
  const me = record.me;
  if (me && typeof me.account === "string" && me.account.length > 0) {
    const role = me.role;
    if (role === "owner" || role === "operator" || role === "viewer") {
      status.me = { account: me.account, role };
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

export const authApi = {
  status: () => fetchAuthStatus(),
  me: () => readJson<ProductMe>("/api/auth/me"),
  login: (account: string, password: string) =>
    readJson<ProductMe>("/api/auth/login", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ account, password }),
    }),
  register: (account: string, password: string, token?: string) =>
    readJson<ProductMe>("/api/auth/register", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(
        token === undefined ? { account, password } : { account, password, token },
      ),
    }),
  logout: () =>
    readJson<{ ok: boolean }>("/api/auth/logout", { method: "POST" }),
};
