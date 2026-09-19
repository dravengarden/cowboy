# Buffered Code read authority

This finite core security boundary extends [Session read routes](plugin-session-read-routes.md)
and [Workspace read scopes](plugin-workspace-read-scopes.md). Communication,
authentication and native ownership remain core mechanisms, not Plugins.

## One original request

All eleven buffered filesystem/Git HTTP readers require a private, non-cloneable
read authority extracted from the already authenticated product request. It
retains the original core HTTP state and original product credential, and is
consumed by one response boundary. Client-supplied IDs, serialized principals,
admin cookies and cached responses cannot construct it.

Before invoking any read setup and again before returning its complete buffered
response, core rechecks that credential and the current user's Session
visibility. Cookie checks retain the original session ID and token hash, expiry
and primary/passkey freshness. Personal tokens retain the original hash; device
and automation tokens retain the original identity and token hash without
replaying their one-use sender proof. All authenticated modes check the enabled
user and original [permission lifetime](plugin-product-permission-lifetimes.md).
Role changes end that observation; restoring or increasing the role cannot revive
or expand an in-flight request. Auth-off mode is only the original explicit
local mode; it is never a fallback for an expired or revoked credential.

The shared credential continuation also backs existing Operator confirmations.
Their Operator minimum and automation-mutation refusal remain distinct from
read policy: Viewers can still read their own or unowned Sessions; Owners can
read others' Sessions only while they retain that role. Existing authenticated
shared-Workspace policy and automation read scopes are unchanged. Credential
evidence never enters a file/diff cache, Machine command, journal or diagnostic.

Lost authentication or an ended permission lifetime yields `401/no-store`;
lost Session visibility yields `404/no-store`. Both replace the entire response, including an old ETag,
`304`, cached/raw body or detailed read error. Original logical/transport
observations are checked around asynchronous credential lookup too; their
existing `410/no-store` behavior is retained. Independent native resource
query/release paths are not converted into Session-path reads.

## Legacy language queries

The four Session-path GET handlers for language diagnostics, hover, navigation
and outline now consume the same private read authority. Their closed query
enum cannot encode open, close, reload, sync or owned-resource commands. Each
handler retains its original Session and connection before asynchronous
credential validation, dispatches on that exact connection and checks again
before returning the whole response. A replacement connection, even with the
same reported epoch, cannot be selected during authorization.

Only an existing Session is accepted: a Workspace file-read scope does not
establish a native buffer. The adapter still requires an already opened source
buffer. Stable response schemas and ordinary Viewer/automation read policy are
unchanged. Resource opening/closing and independent original-ID cleanup retain
their separate protocols; no retry, acquisition or release is added when a
reply is refused. Native navigation can itself acquire destination state; this
response fence neither cancels those already-admitted effects nor proves their
release. It is not owned-navigation acceptance or post-effect recovery.

## Acceptance and limits

Source tests cover logout, personal-token revocation (including no cookie
fallback), disabled users, current role/visibility, Viewer success, explicit
local mode and device/automation proof non-replay. Parked reader tests cover
successful, conditional and error responses and refusal before synchronous
setup. The [connected gate](plugin-code-connected-conformance.md) adds v8 check
21: hold the actual core adapter's file reply, log out that disposable browser
through the real product API, then release the unchanged reply. The response
must be denied without an ETag or another Machine command; the independent
original login remains usable. This uses no production credentials.

The language-query extension requires connected v9 checks 22–25. For each
handler, query an already opened real native source, hold a second actual reply,
log out that disposable login through the product API and require
`401/no-store/no-ETag`. A further revoked request must dispatch nothing; the
independent login must still query successfully, with exactly three total
commands of that query kind and no other command. Source tests additionally
cover the closed wire shape, response-kind mismatch, pre-dispatch and
parked-reply connection replacement, and Session path ABA.

The [accepted Controller rollout](releases/plugin-code-read-authority-2026-09-19.md)
records the old-artifact negative result, all 21 connected checks, complete
source/build gates, actual reader floors and production activation. Its bounded
deployment continuity and later Provider-auth roll are reported separately.
The [legacy-language rollout](releases/plugin-language-read-authority-2026-09-19.md)
extends acceptance to all 25 connected checks and records Controller-only
activation with the 16 original workers retained in its own observation window.

These are checks at finite boundaries, not a continuous principal epoch or an
atomic database/network delivery transaction. Core-observed role ABA is fenced
by the permission lifetime; unobserved database disable/re-enable or external
policy edits are not. Already dispatched reads may execute; previously
delivered browser/cache bytes cannot be withdrawn. This does not add a generic
graph grant, state reader/writer lease, effect fence or independent restoration.
No Plugin/SDK, Machine protocol, native binary, SQL baseline or host policy
changes; only a Controller release is required.
