# Victoria telemetry Plugin

Installable data-only OTLP/HTTP protobuf connector for existing VictoriaLogs,
VictoriaMetrics and optional VictoriaTraces services; **not** a database/service
installer. None is a default Cowboy
dependency. Initially advertised for Linux x86_64, with signed lifecycle and
real loopback HTTP fixtures. A production endpoint or another platform needs
its own acceptance; unit fixtures do not assert a particular database version.

Build from the Cowboy root in the pinned shell:

```sh
nix develop -c just example-telemetry-bundle victoria
```

For an explicitly authorized release, use the existing generic
`plugin-set-published-artifact-url`, `plugin-sign`, `plugin-verify` and
`plugin-publish` recipes with the publisher's immutable HTTPS artifact base,
private signing key, public key and absolute Catalog directory. No
`plugin-bind-runtime` is needed: the signed target matrix has zero executable
components. Publication still requires the canonical
`.agents/skills/release-cowboy-plugin` gates and public artifact verification.

After publishing and refreshing the Catalog, `/admin/releases` offers the
exact version on compatible Machines. The same generic API is
`POST /api/machines/{machine_id}/plugins/victoria` with
`{"version":"1.1.0","digest":"sha256:<composite artifact_digest>"}`.
Use the **artifact digest**, not the package digest. Never use a moving alias.
The normal authenticated Product API can install it directly; an Admin login
is not required. Successful installation returns HTTP 204. Then read
`GET /api/machines/{machine_id}/plugins` and require the selected version and
generation digest to be `active` before enabling export.

Installing alone sends nothing. Copy and edit the example policies outside the
repository, replace both zero digests with the installed generation digest,
and protect each file with mode 0600, owned by its respective service user:

- Controller: select `controller.example.json` through
  `COWBOY_TELEMETRY_PLUGIN_CONFIG` (loaded at Controller startup).
- Selected Machine: place `machine.example.json` at
  `<machine-state-dir>/telemetry.json` (read for each export). Set the actual
  service base URLs. Ports 6302/6301 are examples, not mandated service defaults.
  Omit any unneeded lane (including `traces`); disabled signals stay local.
  A lane may include `"bearer_token":"<private value>"` if required; never put
  real credentials in this repository, package, Catalog, command line or receipt.

HTTPS is required except for literal loopback HTTP. Userinfo/query/fragment,
redirect following and ambient proxy inheritance are rejected/disabled. Base
path prefixes are preserved; signed relative routes are appended. Tokens do
not appear in Machine command payloads, logs or result receipts.

Uninstall uses the existing plan/confirm API (or management page). Local
recording continues. Rollback installs a retained signed Catalog release;
both policies must select that exact generation again. Removing the Machine
policy immediately stops new exports without restarting active sessions.
Requests already in flight can finish within their bounded deadline.

Victoria 1.1 requires SDK 1.8, Machine protocol 9 and payload schema 2. The
signed routes are `/insert/opentelemetry/v1/logs`,
`/opentelemetry/v1/metrics`, and `/insert/opentelemetry/v1/traces`. HTTP 200
partial-success responses are counted, not replayed. This connector does not
install or validate the version of any of the three independent servers.
See [Victoria OTLP support](https://docs.victoriametrics.com/opentelemetry/)
and [VictoriaTraces ingestion](https://docs.victoriametrics.com/victoriatraces/data-ingestion/opentelemetry/).

See [full configuration, limits and diagnostics](../../../docs/telemetry-plugins.md).

## Verify actual delivery

Server health, Catalog availability, Machine installation, Controller admission
and backend delivery are different checks. After activation, submit one bounded
synthetic log, a supported counter/histogram and a sampled span through the
authenticated `/api/telemetry/v1/{logs,metrics,traces}` endpoints. Use standard
OTLP/HTTP protobuf and the normal client authentication; do not send real prompts
or credentials as test content.

Query back the exact synthetic log/trace identities and metric sample from the
three independently operated services. Check Cowboy's local-file failures,
remote per-signal failures, rejected items and export-queue drops. HTTP 200
admission by Cowboy alone does not prove backend delivery, and OTLP partial
success is not full acceptance.

Local recording remains enabled throughout; it defaults to 8 MiB per file,
eight retained files and size/UTC-day rotation under a private `/tmp` directory.
Older files are not uploaded on activation. Each remote signal is independently
optional; omitting its Machine policy lane keeps that signal local. Prometheus
scraping and journald shipping are separate from client OTLP export. See
[the client instrumentation and privacy boundaries](../../../docs/client-opentelemetry.md).
