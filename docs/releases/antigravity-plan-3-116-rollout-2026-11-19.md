# Component release 3.116.0 rollout (Antigravity plan and Gemini cards) — 2026-11-19

Status: Controller and Web on Hawk run Cowboy `5260e799`. The seven 3.116.0
Agent Provider releases are published; Hawk, Falcon and OVH are converged and
`antigravity` 0.2.1 is active on OVH. No Machine release or worker pin change
was needed (`Cargo.lock` and the interface files are unchanged).
macbook-air was not converged.

## Content

- Antigravity 0.2.1: Gemini 3.8 Flash High (default), Medium and Low cards;
  the account plan agy reports (`_meta.antigravity.tier`, `{id, name}` only,
  marked by the 0.2.1 proxy) on the account card and in `accounts.list()`.
- The other Providers take a patch bump for the provider-runtime pin only.

## Gates

| Gate | Result |
| --- | --- |
| `check-compact` at the rebased branch | Passed (exit 0) |
| Code review | Two rounds; all findings fixed or simplified away |
| Catalog readers on an isolated copy, two runs each | Active 633, rollback 632 and candidate each read 83 releases |
| `provider-release-coverage` | Passed, isolated and live |

## Publication

| Plugin | Artifact digest |
| --- | --- |
| claude-code 3.20.48 | `sha256:9cba4786920204af3c0422ac10519f1f3c66fd9b5244692f02a0987d0033131a` |
| claude-deepseek 3.1.66 | `sha256:813c16c52f8ba1e96c5dc647cae94428d5111496aa683d7483fd3c0335ca18b9` |
| codex 3.6.6 | `sha256:c22de3755f724b409f76fc4e01e69a5ea9daa6f27312b9983390823355ce9ba0` |
| codex-deepseek 3.1.66 | `sha256:7cd6c28271faa1a80c62cef5d42dd0b3a30bbe09b9f04d9e90b2e605bb354661` |
| grok 3.2.5 | `sha256:1a9667cc97dc9365a8d7f818f09d8bb00fef9a334d2666bbb1402b2458dc1057` |
| deepseek-harness 1.0.24 | `sha256:700e8c1e56f94fcf09f0712185078843fadb167ef6af94923df3f523f6f4dce0` |
| antigravity 0.2.1 | `sha256:1e427d195b09e0df729b8bcdda369e6dfd16ba4ab4b23351abff48a374f86ba8` |

## Activation

Hawk Web and Controller receipts `succeeded`/`committed`, `published: true`,
`/healthz` ok. Convergence applied every planned upgrade on Hawk, Falcon and
OVH (HTTP 204); `ovh-antigravity-0-2-1-upgrade` left 0.2.1 `active`.
