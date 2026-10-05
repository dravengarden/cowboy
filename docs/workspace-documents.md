# Workspace documents

Draft is an independent document resource with no Machine, Provider, agent
status or Code workspace. Session owns its runtime and conversation. Both
resources appear as ordinary entries in one sidebar and the existing shared
directory tree. Selecting an entry changes the main content without replacing
the App, its sidebar, fold state or workspace navigation.

## Create and edit

Create offers Session, Draft and Folder. All three share the selected directory;
empty means top level. Folder-context creation retains that location when the
variant changes. Session alone requires a ready project and AI installation.
Draft retains the device-local timestamp title and first-focus selection.

DraftEditor mounts the existing PlatformComposerEditor, also used by Session
composers. Its extensions, native input, CodeMirror, Vim/IME behavior and document
undo remain owned by that component. Navigation first flushes locally authored
content and attachments; a failed local save keeps the document open. Remote
convergence remains independent of navigation.
Draft selection is remembered per principal on this device. Legacy #drafts links
select an ordinary entry; they no longer replace the workspace with a library.

Mobile uses the existing left Sessions drawer. Draft disables Agent-to-Code
pager navigation and starts on Agent even if Review was last selected. Desktop
uses the existing keyboard tree, folder actions and collapsed rail. Autosave
does not reorder rows. A principal-owned workspace-order replica preserves
explicit mixed ordering without changing authored document bodies.

## Editing on several devices

Editing on two devices converges without a prompt, following Obsidian Sync's
Markdown merge. Every write records the content it was authored against.
When the server refuses a write because another device wrote first, the
client merges three-way: that ancestor, its own pending text and the newer
server text. It folds every pending write into one write and resends. Edits in
different places all survive. Words, whitespace, punctuation and single CJK
characters are merge tokens. An overlapping region keeps both versions, remote
first, rather than dropping either. Overwritten server states remain in the
recovery history (its latest 30 revisions). Attachments are kept unless one
side removed them. Title and location changes are last-writer-wins. Trash and
restore reapply the local intent.

An open editor folds newer content in place. With no local typing since its
last synchronized content, it adopts the update. Otherwise it merges and
autosave writes the result. Remote edits map the caret, stay out of local
Undo, and wait while an IME composition owns the editor. Each applied
mutation announces the document's metadata on the principal-narrowed
`drafts` sync state. An open document fetches only a revision it has not
seen; the sidebar index adopts the metadata directly. A reconnect resync
carries every document announced in the controller's lifetime. The 10-second
poll remains a fallback.

Only texts too divergent to align at bounded cost (over 1,000 edits per side
at both token and line granularity) are not merged. Then the local text
becomes a separate "(conflicted copy)" draft and the editor shows the newer
document. A pending write from an older outbox has no merge ancestor and keeps
the legacy recoverable conflict.

## Move, copy and recover

Rename and Move use the existing folder-name and directory-picker shells. Move
to the empty directory returns a Draft to top level. Grips reorder between rows
and move into directories. Dropping a Draft on the middle of a Session row
copies its current text and attachments to that Session's unsent drafts. It
retains the source, never sends a prompt, and provides a snackbar with Undo.
Undo removes only the exact unchanged copied row; an edited, sent or scheduled
copy is retained and explained. Add to Session offers a searchable directory tree using the shared workspace
folder projection and order. The Draft’s folder and ancestors open initially;
search reveals matching paths and clearing it restores local folds. Top-level
Sessions remain direct leaves. Keyboard/touch selection copies to the exact
Session identity, including when titles are equal.

Trash keeps a tombstone and offers Undo. The shared sidebar exposes Trash when
it contains documents. Restore is an explicit metadata operation.

## Storage and migration

Draft documents remain in their owner-scoped document replicas. SQLite 0031 and
PostgreSQL 0057 add directory-import identities and principal-owned workspace
ordering; historical migrations remain byte-for-byte immutable. Startup imports
legacy Draft directories into session_folders using deterministic owner-scoped
IDs, preserving duplicate names, hierarchy and document identity. It updates
only document parent metadata; bodies, attachment data, body clocks and history
remain intact. An import ledger prevents deleted imported directories from
returning on restart.

Deleting a shared directory reparents documents to a surviving ancestor or root
in the same storage transaction. The transaction uses the document writer lock
and changes only metadata clocks so concurrent body writes remain safe. Shared
parent validation uses the current owner-visible Hub directory snapshot; this
also accepts directories created locally while their persistence is queued.

## Verification boundaries

Browser conformance exercises the actual App with mixed entries, stable sidebar
DOM, shared editor, folder context, drag-to-Session and exact Undo, plus actual
MobileProductShell selection/pager guards. The older standalone Draft workspace
fixture still exercises editor performance, recovery and extension contracts;
it is not the production navigation path. SQLite/PostgreSQL tests cover nested
imports, owner isolation, repeated startup, folder deletion and preserved body
writes/history. Native acceptance uses an isolated account-free WKWebView
fixture of the integrated Mobile shell; it is not a production login or a
physical WeType acceptance. Pitfall #69 remains open.

## Desktop editor commands

Independent Drafts use the same command host, workspace prefix and actual
editor handle as Session prompts. Mod+S flushes local document persistence;
autosave and background synchronization remain independent. In Queue and
Session Draft edit mode, Mod+S saves changes and closes editing, including the
expanded editor. Mod+Enter sends/queues a new prompt, never commits a row edit.
IME transactions and nested/exclusive dialogs retain keyboard ownership.

The Draft toolbar shows actual scoped/armed shortcut states. Formatting,
attachments, Source, Copy to Session (prefix V) and History (prefix G) have
visible hints. More (prefix M) and the Command Palette expose all formatting
and document actions. Available pane width, measured in rem, controls density
so enlarged fonts retain useful writing space. Shared fixed-size chrome uses
the SurfaceProvider size unit: rem on Desktop, original pixels on touch.
The sidebar Create label and fold target also scale with reading size.

Installable editor plugins share this host: their commands join the same
palette and toolbars in Drafts and Session prompts. See
[editor plugins](editor-plugins.md).
