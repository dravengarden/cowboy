# Session usage refresh and HTTP error details

## Failure

The session information sheet passed its agent Provider ID directly to
`POST /api/usage/:provider`. Codex therefore requested `/api/usage/codex`,
although `UsageService::refresh_provider` looks up account identities and the
Codex contract declares `host.account_usage.provider = "openai"`. Claude and
Grok have the same distinction. The controller correctly rejected the unknown
account with HTTP 400 and the plaintext body `unknown usage provider`.

`SessionProviderUsage` discarded that body and threw a new `Error("HTTP 400")`.
`useNetworkActionState` forwarded its message to the global Snackbar. The toast
could not recover the missing operation or explanation.

The reported 20:37 screenshot matches the `refactor` session's tool event
333840 on 2026-09-15. That interval contains busy lifecycle updates and no agent
error event. The existing client evidence does not retain the failing HTTP
request, so the screenshot alone cannot establish which action was pressed.
The usage-refresh failure above is independently reproducible against the
controller's account-key contract.

## Repair

- Session display and refresh resolve the same account through
  `providerUsageAccount`, using the session's exact Provider version and digest.
  Missing declarations fail locally; they never guess an account or refresh all
  accounts as a fallback.
- The shared usage client preserves GET observation and per-account/global POST
  refresh semantics, including cancellation of initial reads.
- `expectHttpOk` keeps the operation, HTTP status and plaintext or structured
  server explanation. Empty bodies and HTML proxy pages retain contextual
  fallback text. Successful response bodies remain readable.
- Session usage, global usage and usage activity use this error path. For
  example, a rejected refresh can now report
  `Could not refresh OpenAI usage (HTTP 400): unknown usage provider`.

## Verification and delivery

`usageApi.test.ts` reproduces the former rejected request, then exercises the
declared accounts for six first-party agent Providers and an unknown future
Provider. It also checks exact-generation lookup, missing contracts, global
refresh and cancellable reads. `httpResponse.test.ts` covers plaintext, JSON,
empty, HTML, failed body reads, cancellation and successful bodies.

Run the Web typecheck, oxlint, tests and production build inside `nix develop`.
This is a Web-only repair: bump the service-worker version and activate the
immutable `cowboy-web-release`. Installed Mobile clients adopt it through
Update. No controller or Machine restart is required.
