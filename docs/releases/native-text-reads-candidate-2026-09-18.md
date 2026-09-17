# Original native text reader candidate — 2026-09-18

The [finite reader](../plugin-native-text-reads.md) is accepted as source and
immutable candidate evidence. It returns complete original native text through
the existing buffer owner, independently of the removed path and released
parent navigation. It adds no navigation acquisition authority, reload or
second resource lifecycle. Service navigation admission still defaults
`closed`; no production activation, signed Catalog publication, Machine
installation, native-generation replacement or Review destination switch is
claimed.

## Exact inputs

Product implementation and static native build:
`a0fc77496a3b6f31eec620ccf45b0e00bf535a9e`.
Integrated Controller/Machine source:
`9dfb114891830b231ca8b3cde93439820dce7490`.
Final clean harness and complete source gate:
`e5c160693402f43b362ea0ff2e9dabb9f3e612aa`.
The intervening native fixture fix changes only test text construction; the
final harness changes only its response budget and failure-stage reporting.
Neither changes these candidates' runtime reader behavior.

- Controller: `/nix/store/b7jbr17g2qk8h8iqs12p3ijp65jjf742-cowboy-controller-release`.
  ELF: `/nix/store/sn1dmqw3bg2c81hml0m9h32q5cwlcrbc-cowboy-0.1.0/bin/cowboy`.
  ELF SHA-256: `fe09c1dee9331889b46a40f60c3202a628a3518036bf60545d3db74351eff04a`.
  Source manifest SHA-256: `d18d96a13bd0a44f2792bc79fe11ecb5004551fb256c0eecd8cbc697146bf516`.
- Machine: `/nix/store/2nwyvbmnbi7f122xbiwvnaml9cg77cz4-cowboy-machine-release`, protocol 21.
  Release entrypoint SHA-256: `7ffd54a231d02f324bdf0211dc13bd26366952f48fada8ff1705d4e1d3b6edf0`.
  Wrapped ELF: `/nix/store/2a68dni7plxdkf0csxzg8jp7hw6n6m48-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`.
  Wrapped ELF SHA-256: `ba28f93d96e242c6e8c7bbef54ccd78f2e4c1f328e56c4e9eb33c09e1d75f118`.
  Source manifest SHA-256: `af984d3e57742a7c60ceda77c02d00e93f4a11aad8ebe3b7522db4efbebdc8f0`.
- Private Zed adapter `1.13.0`: `/nix/store/cn3dwyrki4pl2s8rf1y12z1w6yjv3c1y-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.13.0/bin/cowboy-zed-adapter`.
  SHA-256: `068578ba50092b9475fe52e363aa41e935b1b833e330dd714d5bd95d72193d26`.
- Unchanged private server `1.0.0`: `/nix/store/lywsfx0zmq03ml9rjlqhhqw9dmp0gdvr-cowboy-zed-server-x86_64-unknown-linux-musl-1.0.0/bin/cowboy-zed-server`.
  SHA-256: `da41ec6baee1cbf714b809fcd912dd200be150c634cbe4a6ec6939e323da8131`.

The actual static pair passed the repository-owned runtime builder's static
ELF and closed-environment probes. All upstream dependency pins are unchanged.
The test-only stdio LSP has SHA-256 `93742240881b5d45acf317a4a9927e8832f3d6110106d49b2bdde747746a1d76`;
it is explicitly configured only inside disposable fixture homes, not a
packaged or ambient runtime dependency. The core adapter SHA-256 is
`99cb56c6bacbd3015901b4669c5ab14257cb40227aaf3286c1526d730f74e550`.

Remote main's Android system-haptic implementation was integrated without
altering its behavior. Its omitted component release metadata was repaired by
appending matrix `3.10.0` and app-shell `1.1.2`. Historical entries and every
Plugin's source/version/component-release pin were preserved by that repair;
no Plugin or other component depends on app-shell. This is not a native-shell
or Catalog activation.

## Accepted behavior

The existing API accepts closed `text` requests against an originally opened
buffer, with complete content identity and a bounded page request. The
adapter compares content, binds the original owner and monotonic revision,
and copies at most 64 KiB from the native rope under the same locks. No page
request performs a native-server command, path lookup, reopen or synchronization.

The typed browser owner retains one read job across at most 65 pages. It
accepts only exact offsets/snapshot/EOF and independently verifies complete
UTF-8 bytes and SHA-256 before returning a genuine content capture. No partial
text reaches a consumer. Cancellation, close or authority loss stops further
pages while draining the admitted request; the 60-second observation budget
does not claim cancellation of the original Service borrow.

Shared native/core/Web fixtures and negative tests cover empty/maximal content,
UTF-8 boundaries, BOM/NUL preservation, edit/undo, same-native-ID owner and
mirror replacement, missing history, malformed/stalled pages, corrupted final
content, unsupported hosts and original cleanup. This is an observation,
not a continuously current native position or a transferable grant.

## Completed gates

`nix develop --option auto-optimise-store false -c just check-compact` passed
on the final clean harness: format, lint, dependency audit, component closure,
isolated Plugin builds, source/contract/type/feature gates, tests, Web production
bundle and optimized Rust builds. Counts include 1,473 full-feature core tests,
365 standalone Machine tests, 26 standalone core-adapter tests, 102 private
adapter tests, 1,748 Web tests and 18 isolated PostgreSQL tests.
The dedicated ignored process gates below were run separately.

The complete gate retains the existing three telemetry lint warnings,
the transitive `spin 0.9.8` yanked-version warning and the Web bundle size
warning. Dependency advisories, bans, licenses and sources checks pass; this
slice does not upgrade or suppress those unrelated inputs.

- `cowboy-source-boundary`: cropped Controller/Machine include the shared
  text fixture and core codec, without importing private GPL implementation.
- `zed-native-navigation-conformance`: the exact static pair reads a two-page
  Unicode target after parent release and source/target removal.
- `zed-plugin-conformance`: a real temporary signed installation retains
  target text reads across uninstall, parent release and path removal.
- Seven isolated Firefox 151.0.1 suites pass **49 cases**: core buffer owners
  (11), context (6), cleanup (7), synchronization (8), Review source (6),
  working diff (6) and document refresh (5). The new text cases exercise actual
  browser WebCrypto, no partial display, corrupted final-page refusal and
  cancelled-page draining. HTTP is fixture-owned; no physical device is claimed.
- `code-buffer-connected-conformance`: **two successful v5 runs**, each
  requiring all **18 checks**, real fixture login, enrolled protocol-21
  Controller/Machine transport and the exact signed temporary native pair.

Both connected runs observe three connections/configurations, nine held real
replies, three deliberately discarded replies and one connection cut. Normal
40-second transport deadlines remain intact. Six navigation executions and
five releases retain their one-use/no-replay meanings. After actual uninstall,
ordinary destination handoff and lost-parent-release reconciliation, two
original-owner text requests read the complete Unicode target after path
removal, with no reopen or selection of another runtime. Replacement connection
and restart refuse adoption. Both fixtures report successful cleanup; forced
fixture teardown remains containment, not product recovery or native close
acknowledgement.

## Audit artifacts and corrected fixture issues

Temporary private audit directory: `/tmp/cowboy-native-text-IBMyHV`.

| Evidence | SHA-256 |
| --- | --- |
| Complete `check-final-source.log` | `b4f046d8e49f80fe50bb778edef56ee18c95768d339f049fffc682bc14aa11c3` |
| Accepted `connected-2.json` | `275cbcd73fb4654ac886a2c1c2eb68a3729b12f00924a30079bac9241fccfbe5` |
| Accepted `connected-3.json` | `ac8b2a8ae34251d7a6cb939ab6f0486bb1da45af20894ac902cc3f0b4dcdb9f8` |
| Static-pair `native-2.log` | `5de6faa4b0833c9341fd83ea0ec92b3bf468074b4dced75f5d9385230fab33c1` |
| Signed-lifecycle `lifecycle.log` | `a6fc29fc0bb1db2a6a114202862f68fba031e9e70b2c4afb46e65443b49554cf` |
| Seven-suite `browser-integrated.log` | `ac98a9a1781a3ef4f17300cef79579d6b15e466dc8ba6e7e8f5c936c104670d8` |
| `source-boundary.log` | `ae2d5cb774f8a73e11ec58beb9ce72d52599d7c09fc32dd2d17ab804329936ea` |

Earlier failed or interrupted attempts are not acceptance. The first native
fixture exceeded Zed's default 20,000 line-length threshold for LSP registration.
Short comment lines now preserve that limit and the exact UTF-8 page boundary;
the nonempty-hover assertion remains required. The first connected attempt
reached the original text read but its shared test HTTP client stopped at
64 KiB before decoding the larger JSON response. Only an original buffer
`POST .../read` with `kind: text` now permits 512 KiB, matching the browser.
A regression test preserves 64 KiB for all other paths/methods/read kinds.
No product limit, native guard, timeout or existing acceptance assertion was
weakened. Final commands keep pinned tools first and append the system Nix
directory only where required.

## Still separate

The intended Web navigation owner and complete destination view must obtain
the exact ordinary handoff, display this verified capture and validate its
location before admitting positional reads. That consumer, global native
pre-acquisition bounds, OS filesystem isolation, supported-device acceptance,
signed rollout and separately authorized Machine maintenance remain unaccepted.
So do abandoned-browser/restart restoration, independently authorized
post-effect recovery, general graph/state leases and the other
[completion exits](../plugin-refactor-completion.md). This closes the complete
native text prerequisite, not the whole Plugin refactor or production navigation.
