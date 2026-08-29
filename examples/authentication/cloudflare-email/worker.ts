/**
 * Security-oriented Cloudflare Email OIDC façade skeleton for Cowboy.
 *
 * It intentionally leaves key custody and durable authorization-code storage
 * to the deployer: use a non-extractable WebCrypto signing key behind a
 * dedicated service binding/HSM and a Durable Object transaction. Never put a
 * private signing key, client secret, magic-link token, or email allow-list in
 * an Authentication Plugin package. The package is public data.
 *
 * Cloudflare Email Service can send only from an onboarded domain and, unless
 * an explicit destination allow-list is configured, only to verified
 * destinations. Keep mailbox mappings in protected storage and normalize
 * addresses before comparison.
 */

interface Env {
  PUBLIC_ORIGIN: string;
  LOGIN_TRANSACTIONS: DurableObjectNamespace;
  EMAIL: SendEmail;
}

type SendEmail = { send(message: unknown): Promise<void> };
type DurableObjectNamespace = { idFromName(name: string): unknown };

const noStore = {
  "cache-control": "no-store",
  "content-type": "application/json",
};

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (url.origin !== env.PUBLIC_ORIGIN) {
      return new Response("origin mismatch", { status: 503 });
    }
    if (
      request.method === "GET" &&
      url.pathname === "/.well-known/openid-configuration"
    ) {
      return Response.json({
        issuer: env.PUBLIC_ORIGIN,
        authorization_endpoint: `${env.PUBLIC_ORIGIN}/oauth2/authorize`,
        token_endpoint: `${env.PUBLIC_ORIGIN}/oauth2/token`,
        jwks_uri: `${env.PUBLIC_ORIGIN}/oauth2/jwks`,
        response_types_supported: ["code"],
        grant_types_supported: ["authorization_code"],
        subject_types_supported: ["public"],
        id_token_signing_alg_values_supported: ["RS256"],
        code_challenge_methods_supported: ["S256"],
      }, { headers: noStore });
    }
    // A production implementation must provide the four endpoints declared
    // above with exact redirect-URI/client registration, state+nonce echo,
    // S256 PKCE, one-time hashed magic links, single-use <=60s authorization
    // codes, rate limits, generic responses, and RS256 ID-token signing. The
    // Cowboy repository keeps this file non-deploying so an example can never
    // become an accidental identity authority with placeholder key custody.
    return new Response(
      "Complete the deployment-specific identity authority first",
      {
        status: 501,
        headers: noStore,
      },
    );
  },
};

/**
 * This placeholder makes the required atomic boundary explicit while still
 * refusing every request. Replace it only with a reviewed state machine whose
 * link and authorization-code transitions are compare-and-swap, terminal, and
 * independently credential-hashed.
 */
export class LoginTransaction {
  fetch(): Response {
    return new Response("Identity authority is not configured", {
      status: 503,
    });
  }
}
