# Session worktree preparation containment

Machine preparation refuses a reused session checkout when the destination is
a symbolic link or not a directory. Its canonical parent must be the canonical
managed worktree root. Matching the source Git common directory alone cannot
prove isolation: a link back to the stable checkout also matches that repository.

Both new and reused worktrees require the canonical selected subdirectory to
remain inside the checkout. Internal directory links remain supported. A reused
checkout is checked before anchoring a detached HEAD on a session branch, so a
refusal preserves the original branch state and dirty files. Failed new
preparation uses the existing cleanup of its newly created worktree and refs.

The regression fixture uses real Git worktrees: a committed external link with
a dirty source directory, an existing link to the stable checkout, a detached
checkout with an escaping selection, and a valid internal link. It checks the
refusals, preserved outside files and dirty work, unchanged detached HEAD, and
successful internal-link reuse.

These are preparation-time checks. They do not pin a directory across later
filesystem replacement, fence independent filesystem writers, or establish
continuous Session/worktree ownership or a durable Session incarnation. Direct
caller-owned workspaces and non-Git preparation retain their existing contracts.
