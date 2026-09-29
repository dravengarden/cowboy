# OVH CLI bootstrap — 2026-09-29

Status: CLI correction published; OVH Machine enrollment and real Grok session
acceptance remain blocked. The OVH VPS and its eventual Machine are permanent.

## CLI correction

Implementation source: `72653b17`, published to main. The complete pinned-shell
`just check-compact` gate and final native review passed. Review findings about
workspace IDs, history/resumption, and asynchronous preparation failures were
fixed before publication.
The new receipt is linked from `docs/INDEX.md`. A separate complete coverage
audit found 160 pre-existing unindexed Markdown pages; this task introduced no
new omission and did not expand into that unrelated documentation cleanup.

The stdio ACP bridge now supports an explicit Machine and registered workspace:

```bash
cowboy serve-acp --daemon-url https://cowboy.stormbird.xyz \
  --provider grok --machine ovh --workspace matrix
```

Use the official device authorization flow before invoking it. Remote creation
uses trusted workspace IDs, scopes history/loading to that Machine/workspace,
waits for preparation and Provider readiness, and propagates terminal failures.
It does not retry creation on another Machine. Initial configuration options
retain their wait so Zed can construct model/effort controls. Local callers keep
their existing directory behavior. See the [integration guide](../integrations/zed.md).

The clean Nix CLI build passed, including its package tests. Artifact:
`/nix/store/whdndg6r27jdrv3cg9narx4wylv12vyh-cowboy-0.1.0`.
The `bin/cowboy` SHA-256 is
`42f1131c947f4b545155df58df799e1eea3cda45d3c662f66763b686bbc82de6`.
Its help exposes both options, and its actual production `operator inspect`
request worked, returning the expected HTTP 404 for the unenrolled OVH target.
The task uses this fixed artifact; no system-wide CLI override, Controller,
resident Machine, worker, or Web activation was performed.

## Permanent Machine bootstrap

The official bootstrap at `527fba89155020ee76792722fc47b1107b3657fc` was installed
on OVH Ubuntu 26.04. Archive SHA-256:
`c42507e65282113838cca5b83ccc2733d99f15de7899462614b98a914a534e83`.
The default Service state path was retained; no shortening symlink or old
Lightsail identity was used.

Native `machine-enroll` issued a one-time ticket for `ovh` / `OVH Matrix`.
Native `register` selected Matrix, Columbus, and Suger workspace roots, but
failed on Service metadata retrieval because the Service hostname resolves
into the private overlay, which OVH has not joined. No local Cowboy Machine
identity or user service was created. A later `operator inspect --machine ovh`
returned HTTP 404. Expired ticket files were removed; no Machine was revoked
or deleted. User lingering was enabled for the eventual retained user service.

The catalog inspection observed signed Grok 3.1.24 and Zed 1.20.0. Neither was
installed on OVH. Re-read the approved catalog before installation, then use
normal installation receipts and credential replication.

Two CLI authorization attempts expired without confirmation. The independent
acceptance browser context and its forwarding connection were closed. No
browser cookie or Provider credential was copied. Start a fresh authorization
request and obtain operator confirmation when resuming.

## Unaccepted boundaries

Stormbird owns the missing managed transport and recovery path. This CLI adds
no Stormbird, SSH-tunnel, or SOCKS protocol to Cowboy. Actual Grok interaction,
SSH from that session, Code surface, reconnect/replay, restart persistence,
resource peaks, fault recovery, and physical-client regression remain pending.
