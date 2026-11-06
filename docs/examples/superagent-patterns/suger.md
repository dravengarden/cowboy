+++
schema = "cowboy.pattern-profile/v1"
id = "workspace.suger"
scope = "workspace"
extends = "global"
guidance = ["AGENTS.md","docs/playbook/00-task-loop.md"]
disable = []

[patterns]
"delivery.procedure" = "patterns/suger-delivery.md"
+++

# Suger workspace profile example

Example template; copy into the owning source repository before importing. Guidance paths resolve in the registered Suger source
repository; pattern paths resolve beside this example profile.

Keep personal supervision configuration in the Suger workspace. Company projects
retain their own guidance and must not depend on this workspace. Use the
existing session-analysis skill to propose improvements.
