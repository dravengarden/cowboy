# Immutable private Session deletion writer acceptance

Project-owned `.#cowboy-deletion-conformance` packages a release-built standalone
Machine-host library test ELF with real broker IPC and private writer checkpoints.
Its metadata is component `cowboy-test`, lane `conformance`, fixture
`session-deletion-private-broker`, reader/writer schema `1/1`. It has no Machine
installer or normal Machine executable. The shipped Machine constructor and
production declaration remain writer zero. Fixture-only Victoria manifests are
added only to its source; Cargo checksums, dependencies and worker pin are unchanged.

Exact independently built fixtures:

- Old source `548497f39208adee8b8901d433abf89baef00be1`, `/nix/store/80gwiw47rag712fnfyziwjbamwbcbl6c-cowboy-deletion-conformance-0.1.0`.
- New source `c1200377c4623dbaf5d5352222afba05efd3c9e4`, `/nix/store/njlxgy7l9sysxv3imgm418dr33k6a188-cowboy-deletion-conformance-0.1.0`.
- Production reader source `0d41edebd42ce891998e577c7605e0e4190a53e6`, `/nix/store/7sqzl815jzdrq4x8mm9r3dm1c0n30ar8-cowboy-machine-release`.

The harness requires distinct clean fixture revisions and ELF hashes. These are
two explicit fixture builds of the current journal implementation, not a general
historical production writer matrix. The new ELF includes the process lock-replacement
regression. The child uses only closed synthetic owners and locally owned temporary
state, private user/PID/mount/network namespaces, cleared environments and separate
logs. No real Controller, Provider account, agent prompt or existing session is used.

Fifteen groups pass: each exact fixture confirms deletion ACK and same-ID dedup
without record/inode replacement, competing-writer refusal and old/new/old reader
reopen retaining bytes; four SIGKILL boundaries at staging/file-sync/rename/directory-sync;
lock replacement without new record/staging effects; and storage failure with
negative ACK, preserved surviving-worker channel, fenced reconnect and both readers'
cold refusal. The expected initial Controller reconnect Replay is consumed before
checking deletion effects. Pre-rename staging is retained without replay; post-rename
publication fences the ID without inventing a positive ACK.

The final bridge's closed `release-writer` mode uses the actual reader's synthetic
Machine/Service identity and namespace. It emits the schema-1 record through real
StopSession IPC without rewriting or replacing seed bytes. Two actual production
reader launches, separated by SIGKILL/reaping, reject the deleted worker, report
writer false and preserve those exact writer-produced bytes.

```sh
nix develop -c just session-deletion-writer-conformance OLD_FIXTURE NEW_FIXTURE READER_RELEASE RECEIPT
```

Complete source gates pass 1,849 all-feature tests (53 ignores), 530 standalone
Machine-host tests (15 ignores), formatting and both packaged Clippy slices.
The installed immutable owner `/nix/store/a4jpv2prm56mvkcfy8f0w969hdm4srmc-columbus-machine-activate-1da44eb/bin/cowboy-release-activate` actually refuses both test artifacts
as foreign component/lane source before target selection/dispatch. Both requests
leave production receipt bytes unchanged; no component activation was dispatched.
Machine PID 3031824, Controller PID 2336663 and
all 13 worker/5 keeper IDs/PIDs/states remain identical. The Web link spelling changed
from a direct store path to the component-profile path between samples; both resolve
to the same immutable SPA `/nix/store/6y1yp4rdnjrxglbg0nd5d2vkvwg4zz76-cowboy-web-0.1.0`. The root reader floor remains identical,
all five HTTP checks return 200 and the namespace contains only `.lock`. Noninteractive
sudo and its recorded sudoers digest remain unchanged. Full identities, observations
and snapshots are in [the receipt](plugin-deletion-immutable-writer-2026-10-04.json).

This completes an immutable private-writer crash/coexistence prerequisite, not
power-loss durability, production writer-release admission, public product deletion,
native resume or device acceptance. The installed owner still rejects production
nonzero writer metadata. Activation/fallback/recovery writer admission and exact
actual production writer-release acceptance remain open; portable committed deletion
admission remains closed. The trusted-administrator scope continues to preserve sudo.
