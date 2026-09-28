# Workspace configuration identity candidate — 2026-09-28

The Machine now ends an advertised root identity when it accepts removal or a
change to the original ID/path binding. Intermediate configurations do not have
to reach inventory subscribers to end the identity. Advertisement captures the
current configuration and its identities under the same registry lock.
See the [contract and limits](../plugin-workspace-configuration-identity.md).

## Candidate

- Clean source: `bc385b9d56a2b04ac37b7bdc6bd0d95bf66ff41c`.
- Machine release:
  `/nix/store/zkhl2njr1yypg1cr7l6fir47mn0rwas7-cowboy-machine-release`.
- Worker generation: `worker-8eede51980be9db15d16`.
- Protocol 22 and existing durable formats are unchanged.
- Current Machine release observed during this task:
  `/nix/store/gh05dprk2zk8wqp6kjsiyn1ss30sdwb5-cowboy-machine-release`, source
  `9ec6d7207f7a520085455c1e1f2a8036fcc278d8`, protocol 21. That source is an
  ancestor of the candidate. No production activation was issued.

## Checks

The actual configuration loader is tested with real directories across removal
and re-addition, path A → B → A and ID replacement, returning even the revision
to its original value without an intermediate advertisement. Old carries are
refused immediately and remain refused after a new advertisement; an unrelated
root retains its original identity. Revision-only and rejected-configuration
changes preserve identities. Existing object replacement still changes no
configuration notification.

The negative substitutes the old identity-retention behaviour at the
configuration boundary while leaving the same tests in place. It fails at
`unadvertised_configuration_aba_cannot_revive_an_original_identity`: immediately
after removal, verification of the original identity incorrectly succeeds.
The correct implementation was restored before committing/building the candidate.

Complete pinned-shell `just check-compact` passed: 1,646 main Rust tests,
387 standalone Machine tests, 26 core adapter tests, 126 private adapter tests,
1,939 Web tests and 19 isolated PostgreSQL tests, plus formatting, lint, types,
dependencies, composition, feature and release builds.

The unchanged v14 connected gate accepted all 34 checks in 294.14 seconds on
the clean candidate source. It used the Machine above with Controller
`/nix/store/3biim9xkgsm8yxv3378vcvi5ibjxj7w0-cowboy-controller-release`, adapter
`/nix/store/nr6sw4wv70ckciq6kkrg718nf0hhsb3b-cowboy-zed-adapter-1.20.0/bin/cowboy-zed-adapter`
and server
`/nix/store/i9yxq38r7pzhdkmp4bq8sypihaqlnxpq-cowboy-zed-server-x86_64-unknown-linux-musl-1.6.0/bin/cowboy-zed-server`.
All five connections completed configuration at protocol 22; fixture cleanup
passed. This is regression acceptance of the existing real-process chain,
not a new connected injection of intermediate configuration ABA.

## Local evidence

| Path | SHA-256 |
| --- | --- |
| `/tmp/cowboy-config-identity-input.json` | `cb223724c044206450d753d90ec568b462f5a9a240fc5275821d72fe18fdb2d4` |
| `/tmp/cowboy-config-identity-check.log` | `84e9e5933d111489e23b5394a6fa164b8665b82b9b5cd7d163d1988b7e2a9cb8` |
| `/tmp/cowboy-config-identity-negative.log` | `d3d4b3744d9e5d0fab682924712a5ab72c0804e04d7ce74eda495cc849a55dbe` |
| `/tmp/cowboy-config-identity-connected.json` | `e67e60ebf659d85c4d0c28e17a8f4e0b451a674bc69812f0d9bdd912390f2909` |

## Remaining boundary

Production adoption is the independently authorized resident Machine maintenance
boundary from `AGENTS.md`, including its worker generation and isolated adapter.
The immutable candidate is ready for that separate transaction after acceptance;
publishing this source alone does not activate it. The component activator is:

```sh
cowboy-release-activate --machine hawk --maintenance /nix/store/zkhl2njr1yypg1cr7l6fir47mn0rwas7-cowboy-machine-release
```

Source regression coverage of intermediate configuration ABA is distinct from
the connected gate's actual root replacement and permission-withdrawal checks.
Neither is production Session/native generation acceptance, a general state
lease, post-effect recovery, or proof that already-dispatched reads were undone.
