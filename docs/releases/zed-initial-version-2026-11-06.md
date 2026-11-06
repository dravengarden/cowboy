# Zed 1.26.0 initial diagnostic versions — Hawk, 2026-11-06

Hawk's gopls v0.22.0 omits the version from diagnostics for `didOpen` version
zero. A direct stdio probe confirmed that it does not advertise diagnostic
pull and does publish version one. Private server 1.11.0 starts LSP document
versions at one and requires the exact retained snapshot before claiming
versioned analysis. Zed's version-zero fallback without a snapshot remains
unknown. This corrects the earlier attribution to pull in the
[1.25.0 report](zed-pulled-versions-2026-11-06.md).

The adapter also pins legacy buffer owners to their original native ID and
canonical target. Reopening refuses a replaced native ID or retargeted symlink;
closing uses the original owner without resolving a deleted or changed path.
Other owners, including Code panes, retain their buffers.

## Immutable release

The signed package was built from clean source
`2c8b4dae13467acad86b383a35a3ccca7afa4066`, integrated through
`962d9d0734803e9715cafe6211df6704bfdf0e30` on main.

| Artifact | Version | SHA-256 |
| --- | --- | --- |
| Adapter | 1.26.0 | `d517bbb4d6fc2fb795cba10f0bf87d4575f0adac4b487f50541d4746f03384ff` |
| Private server | 1.11.0 | `dae5ad2e015babc603643a591dfbfff4a074aa30f549451a09c0d2ea0b45ac93` |
| Signed composite | 1.26.0 | `051777894de243f2d64116e7711f65a7f9388b37f0f9b285713ee7d3b0dfb2c1` |

The frozen source passed `just check-compact`. Adapter tests passed (141;
two opt-in fixtures excluded); the private server passed 65 native tests.
`zed-native-navigation-conformance` and `zed-plugin-conformance` passed for
the exact binary pair. An isolated real-gopls probe returned the expected
`undefined: missing` diagnostic with `document_current`, then released its
original owner. Active and rollback Machine readers accepted the signed package.
Published bytes were fetched and verified against their digests.

## Installation and production observation

Host-managed operation `hawk-zed-1-26-0-converge` returned HTTP 204 with
`applied: true` and no remaining convergence actions. Machine inventory reports
the exact active composite above and installation revision
`installation-708b3c0d3dea7f9086e45df6f6947dfa8aa1a77d88d74c75d03cac277910b4d1`.
The earlier MCP install request was refused by the service-managed membership
guard before installation dispatch; no uncertain dispatched installation was
replayed.

Controller activation transaction `1793955262228919062-1116c56161a7` committed
successfully at 08:54:40 UTC, revision
`1116c56161a7521284186238e7e7593c994750d6`; `/healthz` returned `ok`.
After activation, the production `cowboy.code.diagnostics` read of
`machines/internal/cowboyrelease/release.go` in the original Columbus session
returned `state: document_current` and gopls analysis
`{state: document_current, reason: null}`. This is production observation,
separate from installation and the isolated probe. Empty diagnostics here are
not a successful project check or evidence about dependency/configuration inputs.

A subsequent read after idle release returned both `acquired: true` and
`document_current`. The newly started gopls had the original session worktree
as its cwd; its parent was the installed 1.26.0 private server. SHA-256 of that
running server and its adapter matched both immutable binary hashes above.
This verifies the serving pair, rather than inferring it from the active
installation pointer or from a diagnosis an older runtime could also produce.

No Machine/worker maintenance was used for this activation. Held routes may
still retain older generations until their owners drain; this observation does
not claim every route or Machine now serves 1.26.0.
