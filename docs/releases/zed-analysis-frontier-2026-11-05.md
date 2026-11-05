# Zed 1.22.0 analysis-frontier evidence — Hawk, 2026-11-05

## Why

`cowboy.code.diagnostics` reported every open file as `last_observed`: it could
not say whether a language server had analysed the text the agent was looking
at. Private server `1.7.0` records, for each diagnostics set a language server
publishes into a buffer, the document version Zed mapped it onto, whether the
server named that version, the saved version and disk-based or carried-over
entries, and answers `CowboyAnalysisFrontier` for every server running for a
buffer. Adapter `1.22.0` compares that evidence with its own mirror and the set
it holds and adds per-server `document_current`/`stale`/`unknown` to legacy
`bufferLanguage` replies. See
[20](../architecture/20-agent-code-intelligence.md#freshness-and-coverage).

Owned (lease) reads are unchanged: installed Machines parse them with
`deny_unknown_fields`, so the evidence never appears there. The first candidate
did serialize it there; the plugin conformance caught it before publication.

## Source and artifacts

- Source `4ef2ba9cf` (merged to `main` as `157d39f61`); complete
  `just check-compact` green; adapter crate 134 tests; private server `cowboy_`
  suite 53 tests, including an end-to-end fake-LSP test of versioned,
  unversioned and disk-based publications.
- `just zed-plugin-runtime-build https://cowboy.stormbird.xyz/plugin-artifacts`
  from the clean commit:

| Component                   | Version | SHA-256                                                            |
| --------------------------- | ------- | ------------------------------------------------------------------ |
| `cowboy-zed-adapter`        | 1.22.0  | `26b17494639b2c88c45796d753ed8f66ee7cf158c500d65378f7da0d7617dc93` |
| `cowboy-zed-server`         | 1.7.0   | `43397a3a44fdc41ad5a2e0c46182f286173591785c2577ab44f54ebe4f966c19` |
| Package                     | 1.22.0  | `c75d3275f3070e93f4940dfb363afaf93a403fcf43e97ddc998db371c49dc97e` |
| Signed artifact (composite) | 1.22.0  | `022d96ddd5340085eeeda6c462dcf9b02ac966bba610ad828d8646167b0a6966` |

## Acceptance before publication

| Gate                                                                             | Result                                            |
| -------------------------------------------------------------------------------- | ------------------------------------------------- |
| `zed-plugin-conformance` (signed install, leases, uninstall drain, reactivation) | `released_zed_runtime_installs_and_drains` passed |
| `zed-native-navigation-conformance`                                              | passed, now including the analysis frontier       |

After installation the native gate also asserts the whole chain against the
released server bytes: the fixture LSP publishes versioned diagnostics on open,
and the legacy `bufferLanguage` reply reports that server `document_current`.

Both native gates must run with `CARGO_BUILD_BUILD_DIR` unset and a short
`TMPDIR` (for example `/tmp`): the private server's Unix socket paths live under
the test's temporary root, and a long `TMPDIR` makes the server exit at startup.
The 1.21.0 pair fails the same way under the long session `TMPDIR`.

## Publication and installation

- Signed with `cowboy-first-party`, independently verified, rehearsed on a copy
  of `/var/lib/cowboy/plugins/catalog` (exactly six files added, none changed),
  then published there at `2026-11-05T15:19:39Z`. All three public URLs serve
  the exact bytes.
- Controller `157d39f61` (reads the optional evidence; older adapters omit it)
  activated first: receipt `succeeded`, `committed`.
- `cowboy operator converge --machine hawk --plugin zed --apply`: operation
  `hawk-zed-1-22-0-converge`, HTTP 204, phase `completed`, Machine receipt
  `applied`
  (`installation-05252871c7a6039c6d79a67c93186bb462a2b3fa92900ca1f561e538791de709`);
  inventory reports Zed `1.22.0` active with rollback generation 1.21.0
  (`sha256:74c77d02c57f1f76e3020dccba94b8dcfa5f287ccc6eb486b9af5cf79af451a1`).
  Zero Zed session leases at upgrade; no Machine restart.
- Production `cowboy.code.diagnostics` answers with the new shape (`coverage`,
  per-file `state`); a file no Code pane holds stays `not_open`. Per-server
  evidence appears once a Code pane holds the file.
- Falcon and macbook-air have no Zed installation and were not changed.
