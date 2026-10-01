# Session remote pull request review

Mobile Review has a Remote PR source alongside the local worktree. Associate a
PR number or repository-matching HTTPS URL after selecting an installed review
extension and an actual workspace Git remote. This implementation is read-only:
changed files and text patches, PR state, explicit refresh and a GitHub link.
Comments, CI aggregation, review submission and automatic branch discovery are
separate capabilities.

The session's synchronized Review state retains the extension/view, host,
repository name and immutable repository ID, PR number and source selection.
Local files, tabs and progress remain separate. The binding is a selection, not
an installation identity or authorization grant. Opening it resolves the exact
currently installed extension and validates the repository again. Code bodies
and patches never enter session state or browser persistence.

Workspace payload 2 / Plugin SDK 1.10 adds the closed `pull_request` review
capability. Only the repository-scoped pull-request detail endpoint may declare
it. GitHub 0.2.0 supplies this capability; Web does not dispatch on its Plugin ID.
Schema-one packages remain readable. Ordinary reads omit the new optional wire
fields for older Machines.

Reads retain the original authenticated Code context, Machine connection and
installed dependency checks. The existing filtered `gh` environment executes
only fixed repository-scoped GET requests. There is no checkout, fetch, login,
credential export, branch mutation or language-server operation. Remote patches
use read-only CodeMirror without local file or native buffer reads.

Each read checks PR metadata before and after its 20-file page. Continuations
carry repository ID and a revision covering the PR number, head SHA, base SHA
and base branch. Changed refs, repository identity or file counts refuse the
page. This is optimistic observation, **not** a GitHub cross-request transaction:
the PR files API has no immutable revision parameter. A failed page preserves
the readable page. Refresh explicitly starts a new observation. There is no
polling or automatic document replacement on returning to the visible page.

Bounds: GitHub's 3,000-file maximum, 20 files per page, the command's 1 MiB stdout
and 12-second deadline, the operation's 30-second deadline, and at most 256 KiB
per displayed patch. Only one patch page is retained. Added/deleted totals detect
incomplete patches. Unavailable/binary patches have an explicit message and
source link. Oversized upstream responses refuse the preview rather than
returning partial JSON.

Validation includes response/URL tests, fake-CLI changed-ref and foreign-repo
tests, Hub synchronization/restore tests and six real Firefox checks added to
`just workspace-extensions-browser-conformance`. They exercise association,
CodeMirror, continuation identity, changed-PR refusal and late responses across
session switches. They do not establish private-repository authentication or
physical iPhone performance acceptance.

Deployment requires compatible Controller and Machine readers plus GitHub
0.2.0 installed on the selected Machine. Package publication, Machine upgrade
and Web activation remain separate release boundaries.
