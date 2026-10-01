# Session automatic updates and cached transcript recovery — 2026-10-01

Released source: `e134898396b40a61821f5004808a6eeebc84f1cc`, published on
remote main. Exact component receipts and browser evidence are in
[the release receipt](session-auto-update-2026-10-01.json).

## Behavior

Session actions → Reload now offers **Automatically load compatible Provider
updates when this session is idle**. The switch saves immediately to Controller
settings, is scoped to that session and defaults off. It continues to operate
when the Web client is closed. Every 30 seconds, the Controller considers newer
stable versions already installed on the session's Machine. It reuses the
trusted installed-release resolver, compatibility checks, lifecycle fence,
authentication scheduling guard and native-session resume path. Busy, starting,
stopped and failed sessions are not automatically reset. No new Plugin is
installed by this feature and existing model preferences are retained.

The cached transcript bug came from treating restored replica paint as a live
bootstrap acknowledgement. After an initial failure, the hydrated flag prevented
retries, leaving partial old content and a permanent syncing caption. Recovery
now distinguishes readable cache from confirmed live data. The focused visible
session retries with bounded backoff, malformed empty successful responses do
not confirm it, and superseded requests cannot overwrite newer state. Failure
and retry states offer **Retry sync**. Foreground return also recovers an
unconfirmed cached tail. Draft-source retries keep their separate finite policy.

## Verification

All required `check-compact` stages passed, including 1,685 main Rust tests,
Machine/adapter slices, 1,970 Web tests, PostgreSQL contracts, formatting, lint,
typechecking and release builds. The initial Web run encountered one old
source-shape assertion after the retry refactor; the updated complete Web suite
passed and the remaining PostgreSQL/build stages passed separately.

The real Firefox regression uses disposable IndexedDB and a loopback WebSocket
with the actual product store and cached caption. It restores a 17-minute-old
partial response, injects two HTTP 503s and an empty HTTP 200, and verifies that
the fourth bootstrap restores the canonical complete answer without duplicate
rows and removes the cached caption. No real account or model prompt is used.
Policy tests cover default-off behavior, session isolation, restoration,
disabling, malformed configuration and refusal to downgrade or adopt prereleases.
Existing native identity, prompt race and reload contract tests also pass.

## Activation

Controller and Web activation both report `succeeded` / `committed`, with the
source published. Public health is `ok`; Web version is
`2c686b1c72718ad8d0ac7d8db55e9bd4`, service worker `cowboy-v1784`. The shell
and worker return `Cache-Control: no-store`; an unauthenticated policy PUT
returns HTTP 401. HTTPS checks used the public TLS authority with explicit
loopback origin resolution. All 19 recorded Machine/worker PIDs survived the
Controller restart. No Machine or Plugin release was activated.

No existing session was opted into automatic updates by the deployment. Users
can enable it in that session's Reload dialog after the PWA refresh. Physical
iPhone interaction was not exercised; the regression covers the actual browser
store and rendering path with synthetic transport failures.
