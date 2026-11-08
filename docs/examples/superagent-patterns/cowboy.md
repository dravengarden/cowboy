+++
schema = "cowboy.pattern-profile/v1"
id = "project.cowboy"
scope = "project"
extends = "workspace.columbus"
guidance = ["AGENTS.md"]
disable = []

[replace."delivery.procedure"]
from = "workspace.columbus/delivery.procedure@1"
file = "patterns/cowboy-delivery.md"
+++

# Cowboy project profile example

Example template; copy into the owning source repository before importing. Bind this profile to the registered Cowboy project.
Guidance paths resolve in Cowboy, not in the containing Columbus workspace.

This replaces only the delivery procedure. The global supervisor remains.
Model/account, learning mode, participation and grants belong in Cowboy's
database, not in this file.
