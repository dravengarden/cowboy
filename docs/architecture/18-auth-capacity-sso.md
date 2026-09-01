# Authentication capacity, SSO logout, and automation

Status: normative product-auth policy and admission contract.

Cowboy separates durable authorization, signed-in browser sessions, live
interactive clients, WebSocket channels, and automation. These are different
resources and must not be collapsed into one ambiguous "device" count.

## Resource model and defaults

| Resource | Default | Meaning |
|---|---:|---|
| Authorized CLI/ACP clients per user | 12 | Long-lived, browser-approved public keys with rotating refresh credentials |
| Signed-in browser/native sessions per user | 10 | Stable logical cookie sessions; cookie and Passkey rotation preserve the logical session ID |
| Active human clients per user | 4 | Concurrent logical browser, native, CLI, or ACP clients holding a live lease |
| Active human clients for the Service | 32 | Global human concurrency across accounts |
| WebSocket channels per logical client | 8 | Duplicate tabs or transports sharing one stable client identity |
| Active automation clients | 32 | Separate automation-only pool; disabled until automation is explicitly enabled |
| Active lease / heartbeat / reservation | 120s / 30s / 30s | Failure detection and fair handoff timing |
| Automation credential lifetime | 10 minutes maximum | Short-lived, scoped, sender-constrained access; not a user refresh credential |

The Controller publishes the effective policy through `GET /api/auth/status`
and current account inventory through `GET /api/auth/sessions`. Clients show
configured limits and current counts; they never infer policy from local
defaults.

## Versioned server configuration

`COWBOY_AUTH_CONFIG` schema v2 owns the limits and enforcement mode:

```json
{
  "schema": "dravengarden.cowboy.authentication/v2",
  "password": { "enabled": true },
  "passkeys": {
    "enabled": true,
    "prompt_after_login": true,
    "session_refresh_enabled": true
  },
  "session": {
    "activity_sliding_enabled": true,
    "idle_timeout_ms": 86400000,
    "passkey_max_age_ms": 259200000,
    "passkey_warning_ms": 1800000,
    "primary_max_age_ms": 2592000000,
    "primary_warning_ms": 86400000
  },
  "capacity": {
    "enforcement": "enforce",
    "authorized_clients_per_user": 12,
    "signed_in_sessions_per_user": 10,
    "active_clients_per_user": 4,
    "active_clients_service": 32,
    "websocket_channels_per_client": 8,
    "active_lease_ms": 120000,
    "heartbeat_ms": 30000,
    "reservation_ms": 30000,
    "session_overflow": "revoke_oldest_inactive",
    "active_overflow": "wait_or_reclaim_own",
    "single_session_mode": "off"
  },
  "logout": {
    "provider_logout": "offer",
    "backchannel_logout": true
  },
  "automation": {
    "enabled": false,
    "active_clients": 32,
    "credential_max_age_ms": 600000
  },
  "login_method_order": ["cardea", "password"],
  "providers": []
}
```

Schema v1 remains readable for a staged upgrade but forces capacity into
`observe`. New v2 configuration enforces by default. This prevents a software
upgrade from unexpectedly evicting existing clients before the operator has
reviewed the policy. Reducing a v2 limit does not bulk-kill an over-limit fleet:
existing rows drain, while new admission is blocked until the count returns to
policy.

`single_session_mode: "newest_wins"` is an explicit SSO-style account policy:
a successful fresh login revokes the account's other Cowboy browser sessions.
The default is `off`; enabling it is not a substitute for upstream provider
logout.

## Exact admission and fair waiting

PostgreSQL is authoritative in production and SQLite implements the same
transactional contract for local development and tests. Admission serializes
the global decision before counting, then applies the service limit before the
per-user limit. A reconnect with the same stable client ID renews its own lease
instead of consuming another seat.

At the signed-session limit, a normal fresh login may replace the least recently
active session. If the operator reduced the limit below the current count, new
login is rejected until the account drains. A stable session ID survives cookie
and Passkey token rotation, so revocation, capacity ownership, and logout remain
attached to the same browser session.

When no active seat is available, Cowboy keeps the WebSocket in a non-mutating
waiting state and exposes a fair queue. Users are interleaved before FIFO order
within each user's lane. A released seat is briefly reserved for the next
waiter, preventing a reconnect storm from stealing it. Cowboy never silently
disconnects another account. An account may manually reclaim only one of its
own active clients; the update binds account, client ID, and fencing token in
one transaction. The target connection immediately rechecks the lease and
closes before accepting further mutation.

The WebSocket sends `client_capacity` state for `active`, `waiting`,
`channel_limit`, `lost`, or `unavailable`. Web and native shells render a small
topbar/safe-area control and open management only on demand. Waiting views stay
read-only, cached transcripts remain visible, and running agents continue on
Machines.

## Logout and independent login methods

Each Cowboy browser session records the exact primary method that created it.
Password, Cardea, Google, Apple, and other configured providers remain
independent proof sources even when they map to the same Cowboy account.
Primary reauthentication uses that same method; changing methods requires an
explicit sign-out and fresh login.

Logout has three Cowboy scopes:

- `current` revokes the current logical session;
- `all` revokes every Cowboy session for the account; and
- `provider` revokes only sessions bound to the current external Provider.

Cowboy revokes local sessions and active leases before returning an optional
Provider RP-initiated logout URL. Provider logout policy is `never`, `offer`, or
`always`; a Provider without a signed `end_session_endpoint` remains local-only.
The redirect target is the fixed Cowboy logout-completion route. Cowboy never
constructs an arbitrary return URL from request input.

When enabled, OIDC Back-Channel Logout accepts only a signed Logout Token for
the exact Provider issuer and Cowboy client audience. It requires the standard
logout event, `jti`, bounded `iat`/`exp`, and `sid` or `sub`, rejects `nonce`,
and consumes the Provider-scoped `jti` transactionally to prevent replay. It
revokes only sessions whose stored Provider, issuer, subject, or Provider
session ID match the validated token. Password and other Provider sessions are
not collateral logout targets.

## Automation and test isolation

There is no header, query parameter, username, loopback address, or test flag
that bypasses product authentication or human capacity. Principal class comes
only from a server-issued credential.

Automation is disabled by default. An operator with a fresh human session may
request a short-lived credential for a supplied Ed25519 public key. The token
is bound to request method, path, body digest, timestamp, nonce, credential ID,
and that key. Nonces are single-use. The closed scopes are `api:read`,
`sessions:write`, and `websocket`; automation cannot call account, admin, login,
logout, Passkey, device, or credential-management routes. Issuance is durably
audited without storing the bearer or public key. If the audit cannot commit,
the newly issued in-memory bearer is revoked before any response is returned.

Deterministic tests use an isolated SQLite database, fake clock, fake Provider,
and test-owned account data. They do not contact production, acquire human
seats, or reuse a real username. A production smoke test, when explicitly
enabled, uses a dedicated automation public key and the separate automation
pool; its credential expires in at most ten minutes and cannot be converted to
a user refresh credential. Health checks remain authentication-neutral and do
not impersonate a product user.

## Operator checks

For a policy change, verify all of the following before enforcement:

1. `/api/auth/status` publishes the intended v2 values.
2. Existing sessions and authorized clients appear in account management.
3. Two logical clients with one account renew independently and a fifth waits
   under the defaults.
4. Manual reclaim rejects another user and a stale fencing token.
5. Provider logout leaves password and other Provider sessions untouched.
6. Back-channel replay has no second effect.
7. Automation-disabled returns no credential route, and enabled credentials
   cannot cross their declared scopes or human capacity pool.
