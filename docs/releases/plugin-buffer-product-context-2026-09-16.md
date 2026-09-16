# Core buffer product lifetime — 2026-09-16

Status: published on remote main and activated as a **Web-only** release. The
[core identity integration](../plugin-buffer-product-context.md) ends old buffer
contexts on product-session end, observed identity replacement and permanent
root abandonment. Existing local outbox writers still drain before database
disposal. Ordinary Review has not switched to the new Code facade.

## Source and activation

- Implementation: `5c2f739e453769b646de48fd3697bcfa456e6fe0`.
- Clean integrated release: `127a70778794de6f37b2cbad4559255b93492255`.
  The final merge brings in remote `90bb06f2` documentation only; code is
  identical to the accepted implementation tree, which includes `0886506a`.
- Immutable Web release:
  `/nix/store/pzibmc4axhcyk3f315i81k60bagws2bf-cowboy-web-release`.
- Active Web root:
  `/nix/store/k7camf8n18nvmfff7qhhgf7njkskpzix-cowboy-web-0.1.0`.
- Hawk transaction: `1789523921154109554-127a70778794`.
  It succeeded and committed at **2026-09-16 01:58:41 UTC**, with
  `published: true` and `maintenance: false`.
- Service worker: `cowboy-v1695`; served version:
  `d24e8f07c926dace1b21891675595c91`.
- Ordinary recovery remains the dataset-aware predecessor:
  `/nix/store/cg8hd25cklxfbgd77wkb2bkgi2r2vh3a-cowboy-web-release`.

## Acceptance

The complete pinned `just check-compact` passed both before and after integrating
the upstream implementation changes: 1,341 all-feature Rust tests, 305 Machine,
26 core adapter, 35 private Zed, 1,554 Web and 17 isolated PostgreSQL checks,
plus typecheck, lint, dependency, contract, feature and release-build gates.
The existing 30 ignored Rust and two ignored Machine tests remain ignored;
unit success does not stand in for their independent process acceptance.

Fifteen new product-context tests and one new core session-end test are included
in that Web count. Pinned Firefox 151.0.1 separately passed product-context
**6**, buffer owners **6**, Settings recovery **9**, IDB owners **8** and atomic
outboxes **16**. The product-context fixture uses production lifecycle functions,
React StrictMode and native IDB; its final-save test reopens the original dataset
and recovers the pending mutation after authority has ended. Its SHA-256 is
`b101962e584d7791470855f454a3ec056ed6df64098b1bc3510baef0e6467dc8`.

The actual product-store admission fixture also passed changed-dataset and
missing-subprotocol scenarios. Each retained the durable pending prompt with
zero deliveries; returning the old dataset did not revive an ended owner.
The store fixture SHA-256 is
`9fbbd703f6a706d6be73155cfe013db17f1191fe2d62acf06ddf5887449bebe0`.
These tests use synthetic content and isolated browser profiles, not live
credentials, the password-login UI, native LSPs or physical iPhones.

Local and public HTTP probes matched HTML, service worker, main entry and
changed store assets byte-for-byte against the immutable release. HTML and SW
are `no-store`; hashed scripts are immutable. `/healthz`, `/version` and the
public Hawk deployment-health endpoint passed; the Machine remained online.
Between **01:58:14 and 01:59:06 UTC**, the host closure, Controller/Machine
profiles and receipts, Controller/Machine/Victoria PIDs/start times, all **13**
worker PIDs/start times and failed-unit sets were unchanged.

## Remaining boundaries

No Controller or Machine activation, installed Plugin change, private telemetry
policy, authentication mutation, storage schema or record deletion was performed.
The new facade is not yet an ordinary Review consumer. Native positional
content/anchor semantics, unresolved-resource presentation, Machine/Code rollout,
supported-device acceptance and independent post-effect recovery remain open.
Session-end `drained` proves the registered local barriers, not native release.
The [completion ledger](../plugin-refactor-completion.md) is not closed.

Installed PWAs must accept Update or reload to load these bytes; reconnecting a
WebSocket does not replace the current JavaScript.
