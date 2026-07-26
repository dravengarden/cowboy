# Mobile Code Review

Cowboy Code is a mobile-first, read-only review product. It shares Cowboy's
control-plane connection and active session binding, but owns a separate data
plane. Agent transcript traffic must never block a tree, diff, file, or
language-intelligence response, and Code failures must not affect an Agent
turn.

## Boundaries

- The control plane selects the active session/worktree and owns reconnect,
  reload, authentication, and version state.
- The Code data plane owns worktree snapshots, Git changes, file content,
  syntax data, diagnostics, and navigation.
- Mobile Agent and Mobile Code may share protocol types, cache primitives,
  theming, sheet controls, and connection state. Their screen layout and
  interaction code remain separate.
- The browser consumes a stable Cowboy protocol. It must not consume Zed's
  internal protocol directly.

Zed's remote protocol has the right internal concepts: a versioned worktree
snapshot, stable entry identities, incremental updates, lazy directory
expansion, buffers, Git state, diagnostics, and semantic tokens. It is also a
large private protocol tied to the exact Zed server build. A future
`ZedWorktreeProvider` may adapt those messages behind Cowboy's data-plane
contract. That adapter and the isolated Cowboy Zed server must be pinned and
upgraded together.

Cowboy's Zed instance uses isolated user data, cache, runtime, and server
settings. It may read the repository's `.zed/settings.json` because project
settings describe the worktree. It does not inherit Hawk Zed's UI settings.
Selected language servers and extensions may be shared through an explicit,
read-only package/config source rather than by sharing mutable Zed state.

## Latency model

The useful first screen is Git changes, followed by the worktree root and the
last active file. It must not depend on a complete recursive scan.

1. Fetch a small workspace manifest and change summary.
2. Fetch one directory page at a time. Expanding a row starts an abortable
   request; collapsing it cancels that request.
3. Fetch file metadata before content. Normal files arrive as one compressed,
   cacheable object; large files use line-addressed windows backed by a line
   index.
4. Stream snapshot deltas after the cached first paint. Never resend the full
   tree for a single file change.
5. During browser idle time, preload the root, Git changes, and recently touched
   files within a byte and concurrency budget. Disable speculative content
   preloading on constrained networks.

Tree responses use stable relative paths until the Zed provider supplies entry
IDs. Content responses use a content hash and immutable cache key. Directory
pages carry a content revision and matching `ETag`; browser HTTP storage keeps
them fresh for 15 seconds and may paint stale data for up to two minutes while
revalidating. The in-memory tree paints immediately, then revalidates expired
pages without blanking expanded folders. Explicit refresh uses cache reload.

## Scan policy

The effective exclusion policy is the union of:

1. safe Cowboy instance defaults for generated and metadata directories;
2. repository `.gitignore` and Git exclude files;
3. project `.zed/settings.json` `file_scan_exclusions`;
4. Code-specific user additions.

Project inclusions override only project exclusions, never security checks or
the session worktree boundary. Ignored entries can be revealed explicitly, but
are never speculatively scanned.

The v1 data plane is exposed under `/api/code/sessions/{id}`:

- `GET /tree?path=<relative>&limit=<n>` returns immediate directory children.
- `GET /changes` returns normalized working-tree change records.
- `GET /diff?path=<relative>&context=<n>&showWhitespace=<bool>` creates or
  reuses an immutable unified-diff snapshot and returns its first page.
- `GET /diff?cursor=<opaque>` returns the next page from that exact snapshot.
- `GET /file?path=<relative>` returns bounded UTF-8 source content with an
  `ETag`.

Every response carries `apiVersion: 1`. Paths are resolved from the session,
validated, and contained inside its worktree. Git porcelain and Zed RPC values
never cross this boundary. The legacy session file-tree route remains an alias
temporarily, but the Code frontend uses only the stable namespace.

Diff pages carry a content revision and opaque cursor. The browser appends only
pages with the same revision, so a worktree mutation can never splice two
different diffs together. Initial generation is deduplicated across foreground
and prefetch requests. Server snapshots use a 256 KiB page, a 90-second idle
TTL, and bounded 48 MiB / 12-entry cache; generation is capped at 16 MiB. A
missing cursor snapshot returns `410 Gone`, prompting a clean restart instead
of partial data.

The next data-plane revision adds windowed large-file source reads and
provider-driven worktree deltas without changing the provider-independent
response model.

## Code surface

Use CodeMirror 6 in read-only mode, loaded only after the first file opens. It
already ships with Cowboy and virtualizes the document viewport, supports
incremental parsing, selection, wrapping, gutters, diagnostics, and merge
views. Load language packages on demand and do expensive decoration work only
for visible ranges.

Do not use Monaco for the mobile surface: its editor/workbench capabilities and
download cost are unnecessary for a read-only phone workflow. Do not render a
whole file as highlighted HTML: it creates a large DOM, makes incremental LSP
overlays expensive, and degrades large-file scrolling. Shiki remains suitable
for small immutable snippets, not the primary review surface.

Mobile review defaults to a unified diff with collapsed unchanged regions.
Opening a file switches to its read-only source view while retaining hunk,
diagnostic, and symbol navigation. Editing, terminals, task runners, and Zed UI
settings are deliberately out of scope.
