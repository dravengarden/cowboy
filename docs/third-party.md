# Third-party sources and owned deltas

Cowboy builds some upstream packages from source with its own changes. Each
such package is a pinned upstream release plus an ordered series of
**deltas**, one patch file per feature. Upstream code is not vendored, there
is no fork to maintain, and a delta changes in the same Cowboy commit as the
code that depends on it.

```text
third_party/<name>/
  upstream.json              pinned release, archive digest, consumers
  patches/NNNN-<delta>.patch one `git format-patch` file per delta, in order
```

`tools/third-party.ts` is the only writer of `patches/`. Never edit a patch by
hand; edit the expanded Git work tree and export it.

## Principles

- **Upstream first.** Whenever upstream implements or covers a delta's need,
  the delta is dropped and Cowboy adapts to upstream's behavior. Conflicts are
  resolved toward upstream semantics.
- **Ahead of upstream when needed.** A delta may add what Cowboy needs before
  upstream has it, provided it states when it will be dropped.
- **Always rebase.** The series is replayed onto each new upstream release;
  there are no merges. Each delta stays one squashed commit, so it can be
  judged and dropped on its own; follow-up edits are `fixup!` commits that
  export squashes into their delta.
- **Keep deltas thin.** Put behavior in Cowboy-owned files (launchers,
  bridges, wrappers, configuration) whenever upstream offers an extension
  point. Patch upstream source only where it is unavoidable, and consider
  proposing the change upstream. A package that needs large, long-lived
  divergence is no longer following upstream and needs a different ownership
  decision.

## Manifest

`upstream.json` (schema 1):

| Field | Meaning |
| --- | --- |
| `repository` | `https://github.com/<owner>/<repo>` |
| `version` / `tag` / `commit` | Pinned release; consumers must pin `version` |
| `archive_sha256` | Digest of the GitHub source archive of `commit` |
| `pinned_files` | Files in the source tree whose bytes the build relies on (for example a lockfile) |
| `release_tag_pattern` | Tags `status` treats as upstream releases |
| `consumers` | Plugins pinning the package: `source` builds the patched tree, `registry` uses the unmodified upstream release |

`check` requires every Plugin whose `provider.json` pins the package to be a
declared consumer at the same version, and a package with deltas to have a
source consumer. The files of a package are part of each source consumer's
release input: changing a delta requires a new Plugin version.

## Delta ledger

The commit message of each delta is its ledger entry. It must end with:

```text
Cowboy-Delta: <id>            matches the file name
Upstream: not-proposed | cowboy-specific | proposed <PR URL> | merged <PR URL>
Drop-When: <the upstream change, or Cowboy change, that makes this delta unnecessary>
Tests: <paths of tests this delta adds or changes>
```

A `merged` delta must be dropped at the next upgrade. `cowboy-specific`
deltas implement Cowboy-only contracts that upstream will not take; their
`Drop-When` names the Cowboy change that would retire them.

## Develop a delta

```bash
nix develop -c bun tools/third-party.ts checkout codex-acp
cd dist/third-party/codex-acp      # a real Git repository, ignored by Cowboy
npm ci && npm test                 # upstream's own toolchain and tests
git commit -m "..."                # a new delta, with the trailer block
git commit --fixup <delta commit>  # or a change to an existing delta
cd - && nix develop -c bun tools/third-party.ts export codex-acp
```

`export` squashes fixups, renders the series deterministically, and refuses
commits without a complete ledger. Commit the patches with the Cowboy code
that uses them. `drop <name> <id>` removes one delta from the work tree.

Review a series change as code, not as a diff of patch files:

```bash
nix develop -c bun tools/third-party.ts diff codex-acp --base origin/main
```

It expands the series at both revisions and prints `git range-diff`, so each
delta shows its own code-level change.

## Upgrade

1. `status` lists newer upstream releases and the upstream state of every
   delta (it queries proposed pull requests with `gh` when available).
2. `checkout <name>`, then `upgrade <name> <tag>`. It writes
   `dist/third-party/<name>-upgrade-<tag>.md` with the upstream changes,
   listing first those that touch files a delta changes, and rebases the
   series. On a conflict, resolve it in the work tree and continue the rebase;
   `git rebase --skip` drops the delta being applied.
3. Decide every delta against the upstream changes: **drop** it when upstream
   implemented it, **drop and adapt Cowboy** when upstream covers the need
   differently, or **keep** it (and consider proposing it upstream). Use
   `drop` for each removal.
4. `export` writes the series and records the upgrade: the new tag, commit,
   archive digest and pinned-file digests in `upstream.json`, and a
   `git range-diff` of every delta in the upgrade report.
5. Pin the new version in every consumer and its runtime lock, run the
   consumer's build (which runs upstream's typecheck and full test suite),
   the Cowboy gates and the Provider's remote suites, and record the delta
   decisions in the Provider's dependency audit. Release through the normal
   Plugin lifecycle.

## Build interface

`bun tools/third-party.ts prepare <name> <empty-dir> <cache>` extracts the
digest-verified archive, verifies the pinned files, applies every delta in
order with `patch --forward --fuzz=0`, and prints the manifest and series as
JSON. Plugin builders call it as a process because a Plugin package may not
import repository modules. `check --apply` runs the same preparation as a gate.

## Packages

| Package | Consumers | Deltas |
| --- | --- | --- |
| `codex-acp` | `plugins/codex` (source), `plugins/codex-deepseek` (registry) | See [`third_party/codex-acp/patches`](../third_party/codex-acp/patches) and the [Codex runtime notes](../plugins/codex/runtime/README.md) |
| `claude-agent-acp` | `plugins/claude-code`, `plugins/claude-deepseek` (registry) | None; tracked for `status` and upgrades |
