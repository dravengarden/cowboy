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
The process, licensing, trust, and version-pinning contract is defined in
[14-zed-code-provider.md](14-zed-code-provider.md).

## Latency model

The useful first screen is Git changes, followed by the worktree root and the
last active file. It must not depend on a complete recursive scan.

1. Fetch a small workspace manifest and change summary.
2. Fetch one directory page at a time. Opening the drawer prefetches one shallow
   breadth-first frontier; expanding a row consumes that cache and prefetches
   the next frontier with a bounded concurrency/width budget.
3. Fetch file metadata before content. Normal files arrive as one compressed,
   cacheable object; large files use line-addressed windows backed by a line
   index.
4. Stream snapshot deltas after the cached first paint. Never resend the full
   tree for a single file change.
5. During browser idle time, preload the root, Git changes, and recently touched
   files within a byte and concurrency budget. Disable speculative content
   preloading on constrained networks.

Tree responses use stable relative paths until the Zed provider supplies entry
IDs. Directory pages and saved files form a lazy, partial Merkle graph under
`data_dir/code-cache`: opening the drawer materializes only requested directory
nodes, while opening a saved file below 32 MiB materializes its SHA-256 leaf and
content-addressed blob. Large files remain bounded page reads so navigation
never forces a full-file hash. Directory pages carry a content revision and
matching `ETag`; browser HTTP storage keeps them fresh for 15 seconds and may
paint stale data for up to two minutes while revalidating. The in-memory tree
paints immediately, then revalidates expired pages without blanking expanded
folders. Explicit refresh uses cache reload.

The Hawk-local store uses SQLite WAL for node/leaf metadata and atomic-renamed
CAS blobs. Metadata is a fast invalidation key; a cache miss reads the file once
and hashes those same bytes. Unsaved Zed buffers remain an in-memory Zed overlay
and never enter the persistent graph. The default 2 GiB quota starts eviction
at 85%, drains to 70%, evicts single-hit/cold blobs first, and removes entries
idle for 30 days on startup. Missing, corrupt, or orphaned blobs are discarded
and rebuilt rather than becoming file-load failures.

Mobile's process-local directory cache is independently bounded to 128 pages,
uses access-order eviction, and drops idle pages after five minutes. Browser
HTTP storage remains browser-quota-managed and every response is
revalidation-safe, so no client cache is required for correctness.
The Mobile drawer debounces file queries and caps results at 50, so navigating a
large worktree does not require expanding its hierarchy.

## Scan policy

The effective exclusion policy is the union of:

1. safe Cowboy instance defaults for generated and metadata directories;
2. repository `.gitignore` and Git exclude files;
3. project `.zed/settings.json` `file_scan_exclusions`;
4. Code-specific user additions.

Zed `file_scan_exclusions` are hard exclusions. Gitignored entries remain
explicitly browsable (matching Zed's default project-panel behavior), but are
never speculatively scanned unless `file_scan_inclusions` opts them back in.
An ignored child that is itself a Git worktree becomes a new soft-ignore
boundary, so Columbus project checkouts remain discoverable and preloadable.
Inclusions never override Cowboy's heavyweight defaults, security checks, or
the session worktree boundary.

The v1 data plane is exposed under `/api/code/sessions/{id}`:

- `GET /manifest` returns the provider identity, current Git head, and a small
  worktree revision suitable for conditional revalidation.
- `GET /tree?path=<relative>&limit=<n>` returns immediate directory children.
- `GET /search?q=<query>&limit=<n>` returns gitignore-aware fuzzy-ranked files.
- `GET /changes` returns normalized working-tree change records and the
  revision of that exact status snapshot.
- `GET /diff?path=<relative>&context=<n>&showWhitespace=<bool>` creates or
  reuses an immutable unified-diff snapshot and returns its first page.
- `GET /diff?cursor=<opaque>` returns the next page from that exact snapshot.
- `GET /file?path=<relative>` returns the first 256 KiB UTF-8 source window.
- `GET /file?path=<relative>&cursor=<opaque>` returns the next window only while
  the file identity revision still matches; changed snapshots return `409`.
- `GET /file-raw?path=<relative>` returns the raw bytes of a previewable image
  or SVG with the matching `Content-Type`. Other paths return `415`.

Every response carries `apiVersion: 1`. Paths are resolved from the session and
validated. For a Columbus aggregate worktree, the read-only tree additionally
projects registered `projects/<name>` checkouts from the repository's primary
checkout when that ignored child is absent from the isolated session worktree.
Only names with both `project-defs/<name>/project.toml` and a matching project
directory are eligible; canonical containment is checked again for every page
and file read. Session execution, Git changes, history, and diffs remain scoped
to the isolated worktree. History is a newest-first page of 128 commits;
`GET /repository?after=<oid>` appends the next older page. The Code History
tab loads that page when the list approaches the bottom, with commit-shaped
skeletons instead of a hard cap banner. Git porcelain and Zed RPC values never cross this
boundary. Server handlers depend on the product-level
`CodeProvider` interface for manifests, directory pages, changes, diffs, file
windows, and previewable media bytes. The first implementation reads the local
worktree; a future
version-pinned Zed adapter can replace it without changing browser contracts.
The legacy session file-tree route remains an alias temporarily, but the Code
frontend uses only the stable namespace.

Diff pages carry a content revision and opaque cursor. The browser appends only
pages with the same revision, so a worktree mutation can never splice two
different diffs together. Initial generation is deduplicated across foreground
and prefetch requests. Server snapshots use a 256 KiB page, a 90-second idle
TTL, and bounded 48 MiB / 12-entry cache; generation is capped at 16 MiB. A
missing cursor snapshot returns `410 Gone`, prompting a clean restart instead
of partial data.

Source windows end on a complete UTF-8 and preferably line boundary. The
revision covers the worktree-relative path plus filesystem identity, size,
mtime, and ctime; continuations therefore reject replacement and in-place
mutation without rereading or hashing the whole file. Reads are O(window), use
HTTP revalidation, and cap one preview at 32 MiB.

While Code is visible, Mobile conditionally revalidates the manifest every five
seconds. The initial changes response seeds the same revision, avoiding a
duplicate manifest and Git status request during first paint. A later revision
change refreshes the changes surface and invalidates diff
snapshots without interrupting the file currently being read. Hidden Code
surfaces do no polling. The next data-plane revision can replace this bounded
poll with provider-driven worktree deltas without changing the
provider-independent response model.

## Code surface

Use CodeMirror 6 in read-only mode, loaded only after the first file opens. It
already ships with Cowboy and virtualizes the document viewport, supports
incremental parsing, selection, wrapping, gutters, diagnostics, and merge
views. Load language packages on demand and do expensive decoration work only
for visible ranges.

**A Review swipe that hitchs on source/diff but not on README is a
defect.** Translate a viewport bitmap of the editor, not live
CodeMirror. Do not hide the whole code pane (that flashes) and do not
change `.cm-scroller` overflow on claim (that remasures). See
[`mobile-spatial-presentation.md`](../mobile-spatial-presentation.md).

Source views resolve CodeMirror's language description from the path, including
special repository filenames such as `Cargo.lock`, then load only that grammar.
Plain text paints immediately while the parser chunk arrives; unknown files
remain plain instead of guessing. Unified diffs retain their structural
decorations until the data plane can provide source-aligned syntax spans.

Do not use Monaco for the mobile surface: its editor/workbench capabilities and
download cost are unnecessary for a read-only phone workflow. Do not render a
whole file as highlighted HTML: it creates a large DOM, makes incremental LSP
overlays expensive, and degrades large-file scrolling. Shiki remains suitable
for small immutable snippets, not the primary review surface.

Mobile review defaults to a unified diff with collapsed unchanged regions.
Opening a file switches to its read-only source view while retaining hunk,
diagnostic, and symbol navigation. Editing, terminals, task runners, and Zed UI
settings are deliberately out of scope.

Code Review owns only code-surface typography: its 12/14/16/18 px setting
changes CodeMirror source and diff text without changing Agent or sheet text.
Line wrapping uses the same Review setting from both the settings sheet and a
document-toolbar shortcut.

Git review keeps the complete normalized queue for next/previous navigation but
mounts list rows in 80-entry windows. A near-viewport sentinel extends the
window before the user reaches it, bounding initial Mobile DOM and layout work
even when the server returns the 1,000-change maximum.
