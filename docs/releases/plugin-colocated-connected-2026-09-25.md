# Connected coverage of the colocated topology — 2026-09-25

The connected Code gate only ever exercised **remote** execution. Its Machine
declares an outbound connection, so a read the Controller executes itself had
no real-process coverage at all — despite being the shape the primary
deployment runs, where every session read and all 32 advertised roots are read
by the Controller directly.

v13 adds check 33, which restarts both fixture processes into the colocated
shape and exercises that branch end to end.

## Why this could not have been done earlier

`colocated` used to be derived from the Machine's self-declared
`connection_mode`, so a fixture could have reached this branch merely by
declaring local mode over TCP. That is the
[trust gap](plugin-colocated-permission-2026-09-24.md) itself, not a property
worth building acceptance on: the test would have depended on the very
behaviour that needed fixing. Once colocated execution required an explicit
permission, a fixture Controller naming its fixture Machine became an honest
configuration rather than an impersonation, and this coverage became possible.

## What check 33 asserts

Every assertion is anchored on the relay's **command count**, because identical
bytes prove nothing about which party read them — only the absence or presence
of a Machine command distinguishes "executed here" from "executed there".

- A permitted local Machine's advertised root is read with **no Machine command
  at all**.
- Replacing that directory with a different object at the same path holding the
  same bytes returns `410/no-store` with no ETag — and still no command.
- An explicit inventory refresh observes the live object and restores reads,
  again with no command. A fence is not a lost root.

The root is a third advertised workspace used by no Session, native owner or
installed Plugin, so replacing it disturbs nothing else in the run.

## What it deliberately does not assert

Withdrawing the permission and watching the executor change is **not** asserted
here. Restarting the Controller a second time in quick succession makes the
fixture Machine reconnect repeatedly — an observed 16 connections against 5
configurations — and a flapping fixture would make this gate unreliable rather
than more convincing. The permission matrix, including a remote Machine
claiming local mode and a named Machine that did not claim it, is covered
exhaustively by source tests instead.

The relay still classifies a command for this root, so an unexpected dispatch
would be counted against the assertions above rather than silently ignored.

## Gates

- Connected v13 accepted: **33 checks**, receipt schema
  `dravengarden.cowboy.code-buffer-connected-conformance/v13`, 292.44 seconds,
  four connections with four matching runtime configurations, every negotiation
  at protocol 22, 21 held real replies and seven discarded through their normal
  40-second product timeouts. The colocated root's command kind never appears
  in the recorded counts, which is the check's central evidence.
- Complete pinned-shell `just check-compact`: 1,635 main Rust tests (34
  ignored), 384 standalone Machine tests (4 ignored), 26 core adapter tests,
  126 private adapter tests (2 ignored), 1,933 Web tests and all 18 separately
  isolated PostgreSQL tests, plus formatting, lint, types, dependency,
  feature/build gates and all 86 structural-link vectors.

This is a test-only change. No Controller, Machine, Web or Plugin artifact is
activated by it, and it authorizes no production restart.

## Evidence

Private evidence root: `/tmp/cowboy-colocated.evidence`.

| Evidence | SHA-256 |
| --- | --- |
| `connected-receipt.json` | `4e490410e3e4f0ecdea12c0b607627ff5024a621957a051a6a481d22991b2b3b` |
| `connected.log` | `0d29f7a5c221307222ad434dcf1566b16e13ab7b5658c96f492b17fc0d881e19` |
| `check2.log` | `faa44a70b28f925ee0f492792ed36c60907fd76d30160cb417ddee89eebd0522` |

## Remaining

This closes the coverage gap named in
[Controller-owned identity for locally executed reads](plugin-local-root-identity-2026-09-21.md).
It does not close Machine-owned Session or security-domain identity, state
reader/writer leases, general graph contracts, independently authorized
post-effect restoration, native-generation replacement or supported-device
acceptance, nor does it drive an authenticated read through the deployed
Controller. Those remain in the
[completion ledger](../plugin-refactor-completion.md#code-work-still-required).
