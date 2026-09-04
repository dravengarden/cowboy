# Contributing to Cowboy

Thanks for helping improve Cowboy. Focused bug fixes, documentation repairs,
Provider work, tests, and thoughtful product proposals are welcome.

## Before you start

- Search [existing issues](https://github.com/dravengarden/cowboy/issues) before
  opening a duplicate.
- Open an issue before a large architectural change, new persistence model, or
  Provider-platform contract change so the direction can be agreed first.
- Keep a pull request focused on one behavior or design goal.

## Development environment

Cowboy uses a pinned Nix development shell for Rust, Deno, Node, and project
tools.

```sh
git clone https://github.com/dravengarden/cowboy.git
cd cowboy
nix develop
just install
```

Build everything:

```sh
just build
```

For frontend HMR, run the Controller and Vite server in separate terminals:

```sh
just dev
just dev-web
```

## Project contracts

Please preserve these boundaries when making changes:

- Cowboy is Provider-agnostic. New Provider behavior belongs in versioned Plugin
  contracts rather than Provider-ID branches in the product shell.
- Plugin UI is typed, data-only IR. It cannot inject arbitrary JavaScript, HTML,
  CSS, or DOM behavior.
- Desktop and Mobile share domain state but intentionally keep separate,
  input-appropriate interaction models.
- A Machine owns its worktrees and detached workers; client and Controller
  lifetimes must not become worker lifetimes.
- Consolidated PostgreSQL and SQLite migration baselines are immutable after
  release. Add a new migration instead of editing a shipped file.
- Public documentation and artwork must not contain private machine names, local
  paths, credentials, tokens, internal endpoints, or deployment details.

The normative Provider contract is [docs/requirements.md](docs/requirements.md).
The implementation map starts at
[docs/architecture/00-overview.md](docs/architecture/00-overview.md).

## Verification

Run the complete gate from the repository root inside <code>nix develop</code>:

```sh
just check
```

The gate covers formatting, Clippy, dependency policy, Rust tests, Web
typechecking/lint/tests, Plugin conformance, the product website, and release
builds. A smaller test is useful while iterating, but it does not replace the
complete gate before review.

Add regression coverage for behavior changes. For visual work, include
screenshots for the affected Desktop and/or Mobile surface and verify both light
and dark themes when relevant.

## Pull requests

A useful pull request includes:

- a concise problem statement and the intended behavior;
- the implementation and important tradeoffs;
- tests or other deterministic verification;
- screenshots for user-visible changes;
- migration, compatibility, and rollback notes when the change affects stored
  state, Plugins, Machines, or deployment.

Use clear English for source, documentation, identifiers, and commit messages.
Avoid unrelated cleanup in the same change.

## License

By contributing, you agree that your contributions are licensed under Cowboy's
[MIT License](LICENSE).
