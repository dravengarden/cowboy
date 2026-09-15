# About: compact local recovery and separate telemetry diagnostics

The reported 376 records were unowned pre-upgrade browser states, not telemetry
errors. The old About layout placed their warning directly below managed
telemetry diagnostics and expanded 32 download buttons at once.

Local recovery now belongs to Storage, has a neutral count and starts collapsed.
It offers one native selector, previous/next controls and one numbered JSON
download. A failed export does not hide the inventory. Product-session end
clears the old view and seals pending downloads. Mobile telemetry diagnostics
are explicitly opened; Desktop retains the visible diagnostic workbench. Journal
absence is not presented as evidence that remote export has stopped.

## Release

- Clean source, published on remote main:
  `ec8e800c8bd99fb84b0f4eda28bbd44b02812fe0`.
- Web release: `/nix/store/4pz8pjdzad7lqd3hhmflmv1nymwrn393-cowboy-web-release`.
- Hawk transaction `1789432071233653777-ec8e800c8bd9` succeeded, committed and
  published; service worker `cowboy-v1690`.
- Served Web version: `93b263ba9fe56223c7046f63626d34c5`.
- Ordinary recovery remains the dataset-aware predecessor:
  `/nix/store/7dczgy5nm0j3iz5szpi2in5l1i7djvb8-cowboy-web-release`.

## Verification and boundaries

The pinned toolchain/dependency check, TypeScript check, frontend lint, 1,481
Web tests, browser-runner format/type gates and clean Nix Web build passed. Lint
still reports three pre-existing non-blocking spread warnings in `otel.ts` and
`otelTransport.ts`.

All six real-browser suites passed (50 checks): IndexedDB, atomic outboxes,
Plugin lifecycle, Provider UI, Provider management and Settings recovery. The
new eight-case recovery fixture uses 376 synthetic native-IDB records, React
StrictMode and MUI. At 360px width with 44px button targets the expanded
recovery was 443px high. It checks safe single-flight downloads, unchanged
originals, per-record failure, unavailable/empty inventory and late
unmount/logout results. Its bundle SHA-256 is
`c427db3a556e7f1fe2eccf34618da44bd15b201d526b9b93b282781d7ad0b5e7`.

Local and public HTTP checks compared six files per origin to the immutable
release, including the changed lazy Settings bundle. HTML and service-worker
responses are no-store; hashed scripts are immutable. Both `/healthz` and
`/version` passed and Machine deployment health remained online. Between
08:27:43 and 08:28:09 +08:00 the host closure, Controller/Machine profiles and
receipts, Controller/Machine/Victoria processes, all 14 worker PIDs/start times
and failed-unit sets were unchanged.

No storage format, legacy record, pending operation, private telemetry policy,
Plugin package, authentication state or Machine generation was changed. The
fixture uses Firefox 151.0.1 in an empty isolated profile, not an authenticated
production browser or physical iPhone. Installed PWAs need their explicit update
or a fresh page load; reconnecting a WebSocket does not load the new interface.
