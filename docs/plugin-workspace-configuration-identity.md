# Workspace identity across accepted configuration changes

An advertised root's filesystem object can remain unchanged while its configured
binding is removed, moved or renamed. The Machine must end the original root
identity when it accepts that change, even if inventory publication or a watch
subscriber skips the intermediate configuration.

Previously `WorkspaceConfig::reload` changed only the watched configuration;
`observe_roots` removed identities later, during advertisement. A concurrent
reload could therefore accept A → B → A without the registry ever seeing B.
The original directory handle, path and opaque identity then survived. An
advertisement could also pair a captured old configuration with identities
minted after a newer configuration had already been accepted.

The Machine now serializes configuration loading/publication, identity
retirement, advertisement capture and adapter identity verification with the
existing root-registry mutex. Every successful changed configuration drops
tracked roots whose previous ID/path binding is absent from the new snapshot.
Advertisement takes its own current snapshot under that same lock; callers
cannot hand it an obsolete snapshot. Retirement never mints a replacement.
Only a subsequent advertisement may mint a fresh identity, using the existing
filesystem observation and retained directory handle.

Unchanged ID/path bindings retain their identity. Changing only the declared
revision does not retire them; an invalid configuration is rejected before
either configuration or identity changes. For aliases of one tracked path,
removing any prior binding conservatively retires that path's identity.
The original 256-root handle budget and wire protocol 22 remain unchanged.

Configuration notifications still contain only configuration. A filesystem
object replacement changes its identity without restarting the Code adapter;
a genuine configuration change retains the existing adapter reconfiguration
behaviour. An accepted read already dispatched to an adapter is not undone.
This fence does not create authority or recovery rights.

## Verification scope

Real-directory tests drive the actual configuration loader through removal,
path replacement and ID replacement, then return to the exact original
configuration and revision before any intermediate advertisement. They check
immediate old-identity refusal, a fresh final identity, and independent
continuity for an unrelated root. Other tests preserve revision-only and
invalid-configuration behaviour and the object-replacement notification rule.

This covers configurations the Machine actually accepts. It does not detect
unobserved on-disk edits, make inventory delivery atomic across a connection,
cancel already-dispatched reads, or extend identity to Session worktrees or
security domains. The resident protocol-21 Machine receives none of this
behaviour until a separately authorized Machine activation; publishing a
protocol-22 candidate and passing isolated gates do not activate it.
