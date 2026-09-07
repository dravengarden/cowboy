import type {
  ProviderAuthenticationPresentation,
  ProviderCatalogResponse,
} from "@cowboy/provider-ui";
import { resolveProviderAuthenticationPresentation } from "@cowboy/provider-ui";
import type { SessionMeta } from "./protocol";
import { providerEntryForIdentity } from "./providerCatalogRegistry";

export interface ProviderAuthShortcut {
  providerId: string;
  actionLabel: string;
  message: string;
}

const AUTHENTICATION_FAILURE =
  /authentication required|auth method|credentials?.*(?:missing|expired|rejected)|signed out|entered (?:some\()?crashed\)? before readiness/i;

function authenticationPrompt(
  displayName: string,
  presentation: ProviderAuthenticationPresentation,
): Pick<ProviderAuthShortcut, "actionLabel" | "message"> {
  if (presentation === "api_key") {
    return {
      actionLabel: "Add key",
      message: `${displayName} needs an API key before this session can start.`,
    };
  }
  return {
    actionLabel: "Sign in",
    message: `${displayName} needs sign-in before this session can start.`,
  };
}

/** Turn a session-scoped Provider authentication failure into an actionable
 * notice. Catalog state, rather than error-copy parsing, decides whether the
 * shortcut is offered; parsing only replaces a known downstream crash message
 * with the useful upstream cause. */
export function sessionProviderAuthShortcut(
  notice: { sessionId?: string; message: string } | undefined,
  sessions: readonly SessionMeta[],
  catalog: ProviderCatalogResponse | null,
): ProviderAuthShortcut | null {
  if (!notice?.sessionId || !catalog) return null;
  const session = sessions.find((candidate) =>
    candidate.id === notice.sessionId
  );
  if (!session) return null;
  const entry = providerEntryForIdentity(
    catalog.providers,
    session.provider,
    session.provider_version,
    session.provider_generation_digest,
  );
  if (!entry?.manifest.authentication.required) return null;
  const authentication = catalog.authentications.find((candidate) =>
    candidate.provider_id === session.provider ||
    candidate.authentication_scope === entry.authentication_scope
  );
  if (
    authentication?.authentication_state === "ready" ||
    authentication?.authentication_state === "authenticating"
  ) return null;

  const prompt = authenticationPrompt(
    entry.manifest.display.name,
    resolveProviderAuthenticationPresentation(entry.manifest.authentication),
  );
  return {
    providerId: session.provider,
    actionLabel: prompt.actionLabel,
    message: AUTHENTICATION_FAILURE.test(notice.message)
      ? prompt.message
      : notice.message,
  };
}
