# machine-browser 1.0.7 and OVH Machine host update (2026-11-17)

Status: published from `ec72d930`; installed on OVH.

## Change

Chrome 157 downloaded about 47 MB of Optimization Guide TFLite models
(history embeddings, omnibox, page classifiers) into the profile within two
minutes, despite `--disable-background-networking` and
`--disable-component-update`. The profile is a tmpfs without swap, so the
models stayed resident in every browser, made up 47 of 53 MB of every
snapshot, and ran on visited pages. 1.0.7 starts the browser with
`--disable-features=OptimizationHints,OptimizationTargetPrediction` and
snapshots skip `optimization_guide_model_store`.

Measured with the 1.0.5 runtime in a bubblewrap sandbox (headless, same
switches, wikipedia.org, about 2.5 minutes):

| Variant | Profile | Model store | Browser PSS |
| --- | --- | --- | --- |
| 1.0.5 switches | 53 MB | 46.8 MB, TFLite delegate created | 267 MiB |
| `--disable-features=OptimizationGuideModelDownloading,...` | 53 MB | 46.8 MB (no such feature in 157) | - |
| `--disable-optimization-guide-model-downloads-for-benchmarking` | 6.5 MB | absent | - |
| 1.0.7 switches | 6.5 MB | absent, no TFLite | 254 MiB |

The pinned tmpfs saving (about 47 MiB) comes on top of the PSS difference.

## Gates

| Gate | Result |
| --- | --- |
| launcher `cargo test`, fmt, clippy | Passed (15 tests) |
| `runtime/build.ts` | Xtigervnc rebuilt and digest-verified; archive `sha256:7325d85b…` |
| `runtime/conformance.sh` (pristine ubuntu:26.04) | `ok: true`, 81 checks, none failed |
| `just plugin-check` | Passed |
| sign / verify / publish | `sha256:ec4a8c0f…`, local Catalog receipt written |

## Installation

The first install returned `plugin_sdk_unsupported`: the manifest pins Plugin
SDK 1.16.0 (component release 3.110.0) and OVH's Machine predated it. The OVH
Machine host was replaced host-only with `tools/ovh-machine-host-release.sh`
at `ffa12489` (receipt
`/var/lib/columbus/ovh-machine-ffa124897a55-20261117T063803Z`, 17 workers
retained, accepted). The same operation identity
`ovh-machine-browser-1-0-7-no-optimization-guide-models` then completed and
1.0.7 is active. Existing profiles drop the model store at their next snapshot.
