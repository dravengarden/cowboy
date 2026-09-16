# Claude plan usage — 2026-09-16

Status: the initial **3.1.24** publication below is superseded by **3.1.25**,
which is installed on Hawk and verified against a real Max account. See the
[completed host Operator upgrade](agent-operator-upgrade-2026-09-16.md) for the
authorization entry, live collector correction and final installation receipts.

## Behavior

The collector queries the pinned CLI's native `get_usage` control, the protocol
behind the official Agent SDK's experimental usage method. It projects
five-hour, weekly and model-specific utilization and reset times into the
existing usage cards and account summary. Unknown utilization remains unknown.
No inference prompt is sent. Claude owns authentication and credential refresh.
See the [collector contract and upstream API reference](../../plugins/claude-code/USAGE.md).

Queries share a ten-second deadline and bounded output. Transient failures keep
the last successful snapshot; live session updates no longer change its quota
observation time or erase its refresh error. The existing five-minute automatic
refresh, thirty-second manual cooldown and one-minute transient retry apply.

Implementation: `1e79fde41fe5a0d064e93f741142b26c1c530f59`, published to remote
main. CLI 2.1.272 and ACP 0.77.0 are unchanged from 3.1.23.

## Verification

The pinned-shell `just check-compact` gate passed: 1,320 all-feature Rust tests,
300 standalone Machine tests, 26 core adapter tests, 25 private adapter tests,
1,510 Web tests and 17 isolated PostgreSQL tests. The all-feature suite retained
29 explicitly ignored tests and the Machine suite retained two; the isolated
PostgreSQL and Agent runtime gates ran separately.

Seven dedicated collector tests cover numeric projection, missing/malformed
windows, fragmented native output, unrelated replies, bounded failure/timeout,
process cleanup, authentication errors, closed child environment and diagnostic
redaction. Web tests exercise native quota through the production progress and
top-bar parser. The final collector test/format gate passed separately after
review refinements.

The actual pinned Linux CLI completed the collector query in 615 ms with a fake
API key and an isolated network namespace. It correctly reported that plan
limits do not apply and created no session transcript. This fixture does not
prove a real subscriber's quota. Linux worker initialize/session creation,
3.1.14/3.1.24 generation coexistence and descendant drain passed. Actual macOS
arm64 CLI and adapter probes passed with no Service credentials or prompts.

## Release and activation receipts

- Plugin artifact: `sha256:f3f0b9245fcd7084b41c4d3073206926cf0d8b9db0c9badaf3ed8186aca37919`.
- Package: `sha256:7a270b989754ef380689ce62f1b2a9f9317163767adafdded009aa48f7d83200`.
- Signed host bundle: `sha256:120d958ca6a55bdc078b00f9ee9c15dd5669efb55911be39cf8ddf270094ed54`.
- Publisher: `cowboy-first-party`; independent signature verification passed.
- Catalog receipt: `/var/lib/cowboy/plugin-catalog/receipts/claude-code-3.1.24-f3f0b9245fcd7084b41c4d3073206926cf0d8b9db0c9badaf3ed8186aca37919.json`.
- Controller release: `/nix/store/jps7bbmixfn7mj01v0mcg56x4i51cdfp-cowboy-controller-release`.
- Controller transaction: `1789516150534864897-1e79fde41fe5`, committed successfully.

All five public package/runtime URLs were fetched and their complete bytes
matched the declared digests and immutable caching headers. Actual active,
next-transaction recovery and cold Controller readers accepted the exact
candidate twice before publication and again after Controller activation.
The current Controller revision is the implementation commit above.
`/healthz`, `/version` and `/sw.js` returned HTTP 200; the service worker remains
`no-store`. Hawk reconnected with the same `worker-3a889de3bf203a2378b8` generation.

Evidence is retained in
`/home/draven/tmp/cowboy-claude-usage-research-20260916/`, including the complete
gate log, Linux/Mac receipts, Catalog-reader checks, public URL verification and
Controller activation receipt. Temporary remote probe files were removed.

## Initial activation boundary

At initial publication, the installed Hawk Plugin still resolved to the 3.1.14 artifact
`sha256:7033807a6d08554cf9b12706c03a3da3494c70ac62d4b242810cf116fb70806c`.
The management API returned HTTP 401 to the task, so publication did not install
the Plugin. The subsequent authorized host-Operator rollout removed that blocker
and found that the blanket traffic-disable flag also suppressed native quota
requests. Version 3.1.25 corrects that defect and supplies the verified account
progress. The earlier fake API-key probe did not establish subscriber behavior.
