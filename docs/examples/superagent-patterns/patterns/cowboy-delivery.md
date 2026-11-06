+++
schema = "cowboy.pattern/v1"
id = "delivery.procedure"
revision = 1
role = "procedure"
summary = "Verify Cowboy product delivery with component-specific release evidence."
+++

# Cowboy delivery procedure

Design example; not installed or activated.

Read the registered Cowboy source's current `AGENTS.md` deployment section. For
an authorized completed product fix, follow its owning quality gate, remote main
integration and production activation requirements. Do not repeat routine
approval questions already answered by that policy.

Use the narrowest affected component artifact and the machine-owned activator.
Publication and activation are separate observations; follow the owner's allowed
ordering and retain responsibility for any deployed unpublished commit.

Machine/worker generation maintenance retains its explicit acceptance boundary.
The product-fix default cannot widen it. Research/design tasks and unrelated
changes do not acquire release scope.

Verify the expected commit, component, artifact and target against the actual
activation and relevant health evidence. A successful Web release does not prove
a Controller or Machine update. Unknown activations are reconciled under their
existing operation before retrying.

The original task session executes the procedure; the coordinator checks missing
evidence and follows up within its restricted grant.
