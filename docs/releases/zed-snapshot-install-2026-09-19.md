# Hawk Zed snapshot-lifetime installation — 2026-09-19

Status: the separately authorized **Hawk Zed 1.18.0 → 1.19.0 upgrade is complete**.
Service evidence is `completed`, Machine evidence is `applied`, and current
inventory is active on the exact published digest and new installation revision.
The resident Machine and all 18 original ACP workers remain unchanged. A
separate Controller/Web deployment overlapped this operation; it is explicitly
recorded below, not reported as uninterrupted Controller/Web continuity.

This installs the already signed
[snapshot-lifetime release](native-snapshot-lifetimes-2026-09-19.md). It does not
rebuild, re-sign or republish that release, change third-party pins, enable
private navigation or adopt old native owners. Global resource budgets,
independent recovery and supported-device acceptance remain separate.

## Exact release and one operation

The repository-owned convergence script ran read-only with `hawk --plugin zed`.
Its single step selected the already approved `1.19.0` digest and deterministic
operation ID. The exact selected step was then submitted through the standard
delegated `cowboy operator upgrade`, without re-resolving a moving newest
version at dispatch. No other Plugin was selected.

Preflight verifies the existing `1.18.0` installation revision, zero Zed session
leases, enabled admission and no reconciliation requirement. The selected
Catalog entry must be `ready`, Linux x86_64, and match every release identity
field. Its published envelope is byte-identical to the retained signed release,
and the independent immutable SDK verifier validates it against the configured
`cowboy-first-party` public key. No key rotation, host-delegation rotation,
Service login or credential copying occurs.

| Identity | Exact value |
| --- | --- |
| Plugin / adapter | `1.19.0` |
| Private server | `1.5.0` |
| Composite SHA-256 | `04bec5bfe03fbc6aeabe1701102dbbe24d3840007d7bbdbf1087bee83664279a` |
| Contract fingerprint SHA-256 | `0a51b97cb213887bd9b95a8e52230bcf5172a255e242597180b77a1f9e2a6503` |
| Signed envelope SHA-256 | `e8798220c209f7ca6afe3be5a02982f8ae5e89baa2559316f469f949a0378c05` |
| Operation ID | `hawk-zed-1-19-0-converge` |
| Installed revision | `installation-3b4be39a53e6574ecf5aa314c42fbee764eb5175b91ad2632099e7d70803bdc6` |
| Retained previous digest | `92b1078ab8bd8030cda30f32e13c7c7b355ffbd4e9a9dda0a8085c23c0ef89a1` |

One upgrade call returns HTTP **204**, using an intent saved before dispatch.
The Service row was created at `2026-09-19T03:27:09.378Z` and completed at
`2026-09-19T03:27:18.418Z`. Later observation verifies the original operation ID,
exact version/digest, `problem=null`, Machine `applied` revision and matching
current installation. There is no duplicate install, new recovery ID, database
edit or installation-pointer write. Keeping the prior generation is not an
executed or accepted rollback.

## Overlapping component deployment

The first postflight correctly refuses its original-controller continuity
assumption: the Controller profile changed after the upgrade submission.
The saved response already contains HTTP 204. Work continues with **read-only
observation**, never a second dispatch, and records these independently
committed component transactions from source
`15970ec007548a692972432836394a96d245d1fd`:

- Controller `1789788430371522385-15970ec00754`, committed successfully at
  `2026-09-19T03:27:42.615531974Z`, output
  `/nix/store/rlg49dn64q1588mhn2kb4i2ws2hdqqhc-cowboy-controller-release`.
- Web `1789788476006723226-15970ec00754`, committed successfully at
  `2026-09-19T03:27:56.053844306Z`, output
  `/nix/store/qxj58i675sp574psbw2qk8ch3nqqgx0w-cowboy-web-release`.

Those transactions were not initiated by the Plugin upgrade. The new
Controller reads the durable original installation row and actual Machine
inventory successfully at `2026-09-19T03:28:36.909Z`. Dispatch guards remain bound
to the original Controller; only historical observation permits the separately
recorded committed reader transition. This is not recovery authority.

Postflight distinguishes the observations instead of suppressing the change:

- **19 of 20** original Cowboy PID/start/executable identities remain: the
  resident Machine and all **18 ACP workers**. Only the original Controller
  PID `1051040` is replaced, by PID `1803480` in that separate deployment.
- Machine output remains
  `/nix/store/03li1x7fh85ga3ycqiwwhz47l89af442-cowboy-machine-release`, PID
  `1229854`, generation `worker-795a7ae472286bc7993b`; its component receipt
  is unchanged. No Machine maintenance or ACP worker restart occurs.
- The NixOS host closure and the three Victoria process identities remain
  unchanged, as do all other Plugin installation identities.
- Local/public health and Machine deployment health pass. Local/public SPA
  versions agree on `02a50f7a6a7af406350fca9d1fc73a58`, changed from
  `4bb51ded1747cbc6ffef8a7940736881` by that separate Web release.

The original failed continuity observation is retained. Corrected postflight
accepts only the exact recorded Controller/Web transitions and still requires
unchanged Machine/ACP workers, host and unrelated installations. It does not
claim all processes were retained or restore any old native owner.

The settled observation at `2026-09-19T03:31:44.362Z` repeats those checks,
including unchanged resident Machine, all 18 original ACP workers and the exact
active Zed `1.19.0` installation. A second read-only convergence plan has empty
`steps`, `skipped` and `remaining` lists. It submits no new operation.

## Evidence and scope

The prior release's complete source gate, 48 native tests, exact static pair,
temporary signed lifecycle, 19 connected v6 checks, 24 browser checks and
88-release Catalog floor remain **retained publication evidence**, not new
runs against the concurrently deployed Controller. This installation adds
fresh exact-signature validation, live intent/receipt/inventory observations
and bounded continuity/health checks. No production text mutation or supported
physical-device test is performed. The refactor is not declared complete.

Private evidence directory: `/tmp/cowboy-zed-snapshot-install.NFJysx`.

| Evidence file | SHA-256 |
| --- | --- |
| `installation-intent.json` | `1e9b3d1c76386d0a29aac2d0a812a33daef530ddae13b6946d83ee4ea926d4ed` |
| `installation-response.json` | `489e07ad599c7d12d7eab9541d236d387a35b67693e41c6956abf15a752ed640` |
| `installation-acceptance.json` | `e6769f7a2a66e8b430e618bdd200b477f138728a465da5cf9ce3dc9e831f5700` |
| `audit-before.json` | `ceb038b7cba1d1425e8a1618e834a71c76e8dc5ab61177bb74a2d521d6743754` |
| `audit-installed.json` | `55dbdae009ffbedbbfd6f33c4dc10c96c72a30faabec90e239a5d56347ae73a2` |
| `audit-settled.json` | `40e5b981af35f67f7d1c2a1b16c2f0ea619db8943e0c6344c5fb9af557d43e03` |
| `plan-after.json` | `9ff23b3f48df954228cd8366fba525648e7ea9ce133df2b577ca51f5802cf019` |
