# Bun toolchain and component release 3.45.0 — 2026-10-10

Released from clean source `edc167921f8b9921688719ce2684ad2159c6f832`, which
removes Deno from the repository and cuts component release 3.45.0. The six
Agent Plugins are signed with `cowboy-first-party-v1` and available in the live
Catalog. No Machine installation was requested or performed by this task; the
existing hourly Hawk convergence applies them under its own policy.

| Plugin | Previous | Released |
| --- | --- | --- |
| claude-code | 3.19.15 | 3.19.16 |
| codex | 3.4.0 | 3.4.1 |
| claude-deepseek | 3.1.31 | 3.1.32 |
| codex-deepseek | 3.1.31 | 3.1.32 |
| gemini | 3.1.31 | 3.1.32 |
| grok | 3.1.32 | 3.1.33 |

## What changed in the packages

Only identity. The release exists because the build scripts and tests that
moved from Deno to Bun are part of recorded component and Plugin source
digests; see `docs/plugin-components.md`.

- Every runtime artifact digest equals the one in the Plugin's previous
  published release (2 to 6 components per Plugin, 24 in total). The runtime
  builders, including the Codex adapter's source build, were run on Bun for the
  first time to produce them.
- Each host bundle contains the same files as before; only its Plugin version
  and package digest differ.
- The contract fingerprint differs because the Plugins pin plugin-api 1.1.2,
  provider-ui 3.1.18 and provider-runtime 1.1.13.

## Verification

- `just check-compact` passed on the release commit.
- All six signatures verified before publication. Catalog refresh returned
  HTTP 200 and the Catalog advertises each exact version and artifact digest.
- All 30 published package and runtime HTTPS URLs returned bytes matching the
  SHA-256 digest in their path, through the public hostname.
- `just provider-release-coverage` passed against the live Catalog.

## Not released

- Zed 1.20.6 was built, and its adapter and server passed the builder's
  standalone-executable and probe checks, but it is not published. The Catalog's
  newest Zed is 1.20.2: versions 1.20.3 to 1.20.5 were closure-rule version
  advances that were never released, so publishing 1.20.6 would ship their
  adapter changes without the Zed conformance gates this task did not run.
- GitHub is unaffected by this component release.

## Not checked

- No Plugin was installed on a Machine and no session was started on the new
  releases by this task.
- The Controller and Machines still run hosts built before the Plugin runtime
  moved to Bun. That switch takes effect with their next releases.
