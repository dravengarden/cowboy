# Connected colocation permission withdrawal — 2026-09-28

The v13 gate left permission withdrawal untested after a fixture reconnect
failure. v14 restores that check: keep the same Machine process and its local
declaration, restart only the fixture Controller without its colocation
permission, then require the same advertised file read to cross the relay.

The relay already classified `coreColocatedFile`, but its subsequent identity
allowlist admitted only `coreSwapFile`. A legitimate protocol-22 read of the
colocated fixture after withdrawal therefore failed the relay's own check.
The allowlist now accepts both named advertised-root reads. Regression vectors
retain refusal of identities on Session reads, malformed tokens and other files.
This establishes a relay defect, not the sole cause of the historical reconnect
failure. The connection baseline is also captured before starting the fixture
Machine, so readiness cannot accidentally wait for a second connection.

## Evidence

- Harness revision: `3ceae82e176521703247f99e78cfbb6896e04ebd`.
- Supplied immutable Controller and Machine source:
  `8aed5e11cd9a9e1f08200fac53ae113574b7d489`.
- First v14 run: 34 checks accepted in 293.90 seconds; five connections and
  five configurations, all protocol 22; 21 held replies, seven discarded
  through normal product timeouts; fixture cleanup accepted.
- Repeat run on the same clean harness revision: 34 checks accepted in 293.94
  seconds, again five connections/configurations at protocol 22, identical
  colocated dispatch counts and successful cleanup.
- `coreColocatedFile` totals two commands: a readiness read after withdrawal
  and the measured read. The measured read adds exactly one command and no
  other command; the preceding permitted-local reads add none.
- Complete pinned-shell `just check-compact` passed: 1,644 main Rust tests,
  385 standalone Machine tests, 26 core adapter tests, 126 private adapter
  tests, 1,939 Web tests and 19 isolated PostgreSQL tests, plus the gate's
  formatting, lint, dependency, type, feature, composition and build checks.

Inputs:

```json
{
  "schema": 1,
  "controller": "/nix/store/3biim9xkgsm8yxv3378vcvi5ibjxj7w0-cowboy-controller-release",
  "machine": "/nix/store/vlb8lm8q2qj9spyvl03sjf1lh6z4nc6s-cowboy-machine-release",
  "adapter": "/nix/store/nr6sw4wv70ckciq6kkrg718nf0hhsb3b-cowboy-zed-adapter-1.20.0/bin/cowboy-zed-adapter",
  "server": "/nix/store/i9yxq38r7pzhdkmp4bq8sypihaqlnxpq-cowboy-zed-server-x86_64-unknown-linux-musl-1.6.0/bin/cowboy-zed-server"
}
```

Local evidence SHA-256:

| Path | SHA-256 |
| --- | --- |
| `/tmp/cowboy-withdrawal-input.json` | `d521338f1530ac5fc753e7686c69ea595e83f9b34ad001b40c8d144060191075` |
| `/tmp/cowboy-withdrawal-run1.json` | `94e6f9b89858c4f9b7b433ffa8ce57bb8cfa8999cf8c26d2e1a29ea4b59242f6` |
| `/tmp/cowboy-withdrawal-run2.json` | `532801007a216769e54348f55f0ff310a9125778fd83d91f249ef83333f9fe7c` |
| `/tmp/cowboy-withdrawal-check.log` | `c0ae85ed4ad8b6806f189abef4a142d095a96c9ce3d8a247dbe2f789c51f71bd` |

This is source-only test delivery; no production component or Plugin activation
is needed. It closes the withdrawal coverage gap in the
[previous acceptance](plugin-colocated-connected-2026-09-25.md), not production
authenticated-read acceptance, general state leases, Session/security-domain
identity or independent post-effect recovery. Historical v13 receipts remain
unchanged and do not accept check 34.
