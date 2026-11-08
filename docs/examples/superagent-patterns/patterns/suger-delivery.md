+++
schema = "cowboy.pattern/v1"
id = "delivery.procedure"
revision = 1
role = "procedure"
summary = "Follow Suger task owners and keep company publication within user approval."
+++

# Suger delivery procedure

Design example; not installed or activated.

Read the registered Suger source's `AGENTS.md` and
`docs/playbook/00-task-loop.md`, then the target project's nearest guidance.
Treat workspace policy and the project's requirements as constraints, not
permission borrowed from another workspace.

For company-project work, use the existing task-workflow, local-e2e,
review-branch and sync-pr-branch owners as applicable. Prepare the selected
change, relevant verification and reviewable evidence. Read completed review
results before claiming a review outcome.

A local result, review or pattern match does not authorize push, PR creation,
shared evidence publication or deploy. Reuse the user's existing explicit
authorization when it covers the concrete action and result. Otherwise show the
prepared result and the outstanding decision once.

For an authorized shared dev deployment, follow the existing owner procedure and
its fresh-main, clean/published commit and environment-claim checks. Do not
reconstruct it from ad hoc commands or copy host paths into company
repositories.

Completion distinguishes implementation, validation, publication and deployment;
only the milestones required and authorized for this task must be satisfied.
Unknown or mismatched SHA/environment evidence stays incomplete.
