# Session deletion namespace path identity

Preparing production writer admission exposed a reader/writer ownership bug:
`Journal::open` canonicalized the namespace before opening it. This followed a
final namespace link despite the subsequent `O_NOFOLLOW`, and retained the
resolved target instead of the caller's logical namespace path. Replacing a
parent alias could leave admission attached to an obsolete target.

The journal now makes the caller's path absolute without resolving symlinks and
opens that namespace with `O_DIRECTORY | O_NOFOLLOW`. Initial final-component
links refuse before lock creation, record loading or broker binding. Retained
namespace checks still compare the logical path's directory device/inode and
lock identity to held descriptors. Parent aliases remain valid initially;
observed replacement ends reader/writer admission before a later journal write.
This is the existing observed-identity boundary, not a race-free same-user or
administrator mutation fence. The writer remains hard-coded off in production.

New unit fixtures cover both reader and private writer final-link rejection and
parent-alias replacement, retaining external bytes and forbidding new records.
Disposable broker processes exercise both reader/writer startup refusals before
Welcome/socket creation. The opt-in `session_deletion_releases` test runs exact
independently supplied old/new immutable Machine binaries with linked empty and
committed namespaces. The old reader must admit and create a target lock; the new
reader must refuse without creating a target lock or changing record/link bytes.
The complete existing 31-process upgrade/rollback/refusal matrix must also pass.
Source gates, native identities and activation receipts follow below.
