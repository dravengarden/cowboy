# GitHub workspace extension

GitHub 0.2.0 contributes Pull requests, Issues and Actions to Cowboy's generic
Extensions workbench. It uses an existing `gh` login belonging to the workspace
Machine's OS user. The initial release targets Linux x86_64.

No OAuth token, repository URL, JavaScript or executable is packaged. The host
resolves the selected Git remote and uses `gh auth status --active --hostname`
and bounded `gh api --method GET` calls. The package declares repository-scoped
endpoints and field projections in `contract.json`. Private resource bodies
are not persisted in Cowboy history or client storage.

`gh` is an explicit user-managed connection capability. It is not an Agent
dependency or Provider authentication replica. Missing CLI/login/access appears
as a connection error; the extension does not start a login or change accounts.

Payload 2 adds read-only PR review for Mobile sessions, requiring SDK 1.10.
See [remote PR review](../../docs/remote-pr-review.md). It grants no GitHub writes.

Build from the repository's pinned shell:

```sh
nix develop -c just plugin-build github
nix develop -c just plugin-isolation-check github
```

The SDK also builds this directory independently using `cowboy-plugin-pack build`.
The resulting package uses release schema 3, a complete empty runtime matrix,
and Plugin SDK 1.10. Publish it through the ordinary artifact-URL, sign, verify
and immutable Catalog commands. Readers of schemas 1–2 skip the outer envelope.
Installation is a separate exact-release operation on a compatible Machine.

See [the extension design](../../docs/workspace-extensions.md) for the generic
contract, runtime dependency graph and UI. This package needs repository context
but no language server, so it has no artificial dependency on the Zed runtime.
