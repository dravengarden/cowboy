+++
schema = "cowboy.pattern-profile/v1"
id = "workspace.columbus"
scope = "workspace"
extends = "global"
guidance = ["AGENTS.md","docs/requirements.md"]
disable = []

[patterns]
"delivery.procedure" = "patterns/columbus-delivery.md"
+++

# Columbus workspace profile example

Example template; copy into the owning source repository before importing. Guidance paths resolve in the registered Columbus source
repository. Registered external projects keep their own delivery ownership.

A project-specific procedure replaces this routing procedure explicitly, as in
the Cowboy example. This selection cannot grant machine deployment authority.
