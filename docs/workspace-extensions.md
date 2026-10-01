# Workspace extensions

Workspace extensions add repository resources to Cowboy's Code / Review
workbench. They use the existing signed Plugin Catalog, exact Machine
installation, upgrade, rollback and uninstall lifecycle. `workspace_extension`
is a capability of that lifecycle, not a second extension marketplace.

The Zed engine continues to own code intelligence and its isolated runtime.
Extensions consume a core-owned workspace context; an extension that actually
needs an engine declares an exact Plugin dependency. GitHub needs a Git remote
and a GitHub CLI session, so it does not require a running language server.
This is not compatibility with Zed's native WASM extension ABI.

## Contracts and composition

A signed, data-only contract declares identity, label, description, exact Plugin
dependencies (version and artifact digest), supported Machine platforms, a closed connection capability and
resource views. Views declare a repository-scoped GET endpoint, bounded page
size, filters and JSON-pointer projections into Cowboy's resource model. Detail
views use the same model with Markdown, metadata and an HTTPS source link.
No package supplies JavaScript, CSS, HTML, shell commands or arbitrary network
targets. The shell never dispatches on `github` or any other Plugin ID.

Component dependency closure remains a build-time rule. Runtime Plugin
dependencies are resolved independently on the workspace's Machine, checking
cycles, exact versions and verified installed releases. No automatic install
or implicit upgrade completes an unresolved graph. Each request names the
extension's exact version and digest; dependency changes invalidate the result.

## Connection and authority

The first connection capability is `github_cli`. It is a host-mediated port to
an existing user-managed CLI session, not an Agent Provider login or a private
executable shipped by the extension. Cowboy invokes `gh api --method GET` on
the workspace's Machine. It never exports tokens, copies login files, starts
login/logout, runs `gh` on the Controller for a remote workspace, or accepts
browser-supplied argv, an endpoint, credentials or a filesystem root.
The Nix Machine package supplies the CLI in its service PATH; the OS user's
existing CLI login and configuration continue to own the connection.

The selected repository comes from an actual Git remote. HTTPS and SSH GitHub
remotes are normalized to host/owner/repository; hosts need an existing CLI
session. Requests remain confined to `/repos/{owner}/{repo}/…`. The host owns
environment filtering, process cancellation, deadlines, output and concurrency
bounds. Raw stderr, authentication state and unprojected API JSON do not cross
the Machine boundary. Read failures are typed and retryable without side effects.

Requests retain the existing Code context's original authenticated connection,
workspace identity and product permission observation. Installation and
dependency identities are checked before dispatch and before exposing results.
Private resource responses use `no-store` and are not put in durable inventory,
telemetry, conversation history or service-worker caches.

## Workbench UI

The workbench has one Extensions entry. It opens a searchable installed
extension picker, keeping navigation usable as the collection grows. Inside an
extension, a repository selector and view picker lead to common list, filter,
paging, refresh and detail primitives. Desktop uses available pane width;
mobile uses a full-height touch surface with explicit back navigation.

GitHub 0.1.0 supplies Pull requests, Issues and Actions. Items open readable
details in Cowboy and an explicit source link opens GitHub. Loading, empty,
offline, missing session, unavailable dependency and access failures belong to
the host renderer. Manage opens a common Machine selector and exact-version install, upgrade and
uninstall controls inside the workbench. It starts on the workspace’s Machine.
Management uses the common Plugin compatibility contract for platform, SDK and
schema requirements. New installations start with the newest compatible release;
existing installations and explicit choices retain their exact digest even when
the Catalog changes. Version choices belong to their selected Machine. Catalog
recovery is separate from installation failures, and the shared operation history
provides read-only receipts without repeating an install or uninstall attempt.

## Acceptance

Exercise package validation, malformed projections/endpoints, dependency cycles
and version mismatch, signed installation and removal, repository normalization,
same-Machine routing, bounded CLI execution and safe error projection. Fake CLI
fixtures contain no credentials. Verify actual `gh` access read-only only after
the user has authorized reusing that Machine's CLI session. Run frontend type,
lint and resource-navigation tests, the repository gates and immutable builds.

References: [Zed extension capabilities](https://zed.dev/docs/extensions/developing-extensions),
[GitHub CLI API](https://cli.github.com/manual/gh_api),
[GitHub CLI authentication status](https://cli.github.com/manual/gh_auth_status).

## Reader compatibility

Payload 2 adds a closed remote pull-request review capability (SDK 1.10).
See [session remote PR review](remote-pr-review.md) for selection, snapshot,
resource-bound and compatibility behavior.

Workspace payload 1 requires Plugin SDK 1.9 and outer release schema 3. The
complete runtime matrix contains no executables. Retained schema-1/2 Catalog
readers skip this envelope before decoding its new capability. All existing
Agent, Authentication, Code and telemetry envelopes retain their schema rules.

Publishing does not install a new kind. Installation journals and Machine
inventory also carry Plugin kind, so active, recovery and cold readers must all
understand `workspace_extension` before enabling its first production install.
An old reader's successful Catalog skip establishes publication compatibility,
not compatibility with a populated extension installation journal.
