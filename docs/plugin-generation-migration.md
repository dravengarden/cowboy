# Broken Plugin generation migration preparation

Status: migration preparation with Cowboy-side guards and isolated diagnostics,
not a release-policy waiver or authorization to modify a live Catalog, Machine,
credential or session.
The user requested continued work after rebasing onto remote main on 2026-09-07.
The normative [requirements](requirements.md), especially CR-3, CR-7, CR-8 and
CR-9, continue to apply.

## Established baseline

All five published Codex DeepSeek generations (3.1.2, 3.1.3, 3.1.6, 3.1.7,
3.1.8) fail isolated exact-package startup before native allocation. Their
archived adapters drop the signed configuration arguments. The old 3.1.13
candidate passes alone; its normal coexistence attempt stops before the older
worker becomes ready. These results do not establish the health of live
sessions. Exact identities and immutable evidence are in
[the handoff](../PLUGINIZATION-HANDOFF.md).

The post-rebase component release 2.7.0 has new candidate identities, including
Codex DeepSeek 3.1.14. Previous receipts remain historical evidence, not
acceptance of these newly built packages. A newer name alone cannot resolve the
broken predecessor gate.

At clean source `bb69829b9cde2e3782326b3ac80bf7177d0127e1`, the new exact 3.1.14
candidate passes Linux x86_64 single-generation startup and drain. Its normal
3.1.8 coexistence attempt still fails before the older worker becomes ready.
Separate candidate-first failure-isolation diagnostics pass against all five
published predecessors: their startup rejection and fixture cleanup leave the
candidate and its sidecar healthy. The handoff records the exact worker,
composite identities and seven new private evidence hashes. These observations
establish neither healthy coexistence nor migration of an existing session.

## Separate the questions

| Question | Required evidence | Authority granted by this investigation |
| --- | --- | --- |
| Can the new exact runtime initialize and stop? | Normal single-generation worker receipt | None for production |
| Can two healthy generations coexist and drain? | Normal `--previous` worker receipt | None for production |
| Does an old startup rejection leave the new worker intact? | Failure-isolation diagnostic with both exact identities and verified cleanup | None for publication or migration |
| Can an existing session resume on an installed replacement? | Native ID/history, workspace, auth-home, preference and queued-input preservation; visible failure recovery | None for live rebinding |
| Can a defective generation stop being offered for new work? | An exact-identity lifecycle policy, compatible readers, rollback and retained-session tests | None for live retirement |

The owned failure-isolation diagnostic starts the candidate first and probes
its declared sidecars. The previous exact worker must explicitly reject startup
before any native ID is observed; its recorded descendants must terminate.
The candidate must remain alive with healthy sidecars, then stop and drain.
Unexpected readiness, timeout, malformed handshake, artifact failure, failure
after native allocation or cleanup leakage rejects this diagnostic. Its receipt
has a distinct schema, `release_accepted: false` and coexistence `not_proven`.
No exception text, logs, credentials, private configuration or model prompts
enter that receipt. Normal coexistence acceptance remains unchanged.

## Cowboy-side transition checks

Continued review found and reproduced two gaps in the existing reload path:
unacknowledged old-release setting commands could survive a cross-version reset,
and a snapshot with the wrong Provider version/digest, authentication generation
or requested native ID could acknowledge `EnsureSession`.

Cross-version reset now discards only that session's pending setting commands,
keeps its durable preferences, and rebuilds supported settings from the new
worker's option snapshot with fresh command IDs. Unsupported preferences stay
stored but are not sent. Other sessions and same-release reloads retain their
pending settings. Snapshot acknowledgements require the exact Provider/auth
identity and any requested native ID, including when a busy worker is allowed
to drain on an older Cowboy worker generation. The Supervisor test now uses an
actual recorded auth generation (7), distinct from the proposed current
generation (999), to verify that reload retains the original runtime-home
identity.

Six bounded in-memory ACP byte-stream tests drive the production `run_session`
and notification handlers against a deterministic peer and real Cowboy Hub:
resume is preferred over load; load replay does not duplicate saved messages;
fresh post-load messages are accepted; failed resume/load never falls back to
new; absent resume support fails before session creation; genuine new sessions
can still allocate an ID. Callback barriers, rather than sleeps, ensure replay
actually occurs while load is pending.

The 2026-09-08 completion review adds three resumed-turn byte-stream tests:
the peer withholds model and reasoning replies while a prompt is queued;
no prompt RPC is emitted before both authoritative selections arrive. Rejected
configuration blocks that prompt, while Cancel stays responsive. Controller
wire tests also retain the first cross-version prompt until the replacement
advertises its own vocabulary (including an empty list), filter stale choices,
preserve unsent input on startup failure, and cancel held input without sending
it after startup. Standard, synthesized-mode and Grok configuration paths all
carry completion fences; queue acceptance alone never releases a prompt.

These tests establish Cowboy protocol behavior, not restoration of a particular
upstream CLI's on-disk native history, complete Controller/Machine installation,
or release acceptance. The real upstream native-history acceptance remains a
separate requirement. The signed 3.1.14/1.2.2 production installation is recorded
in [the publication receipt](releases/plugin-publication-2026-09-07.md), not in
this earlier preparation snapshot.

## Proposed transition using the existing lifecycle

1. Build and validate a new immutable candidate without changing historical
   archives, release envelopes, signatures or trust. Do not publish it while the
   current required release gates remain unresolved.
2. Establish a separately reviewed broken-baseline release/retirement policy
   before treating any diagnostic as sufficient for a transition. No implicit
   exception is implemented here. Avoid a Provider-ID special case or a second
   release lifecycle.
3. In disposable fixtures, inventory sessions by exact Machine/Plugin/digest,
   native ID and auth-generation ownership. Distinguish a rejected registration
   with no native ID from an established session and from a running turn.
4. Use the existing explicit idle-only **Load installed Provider** path for an
   established session only after its exact trusted replacement is installed
   and native resume compatibility is verified. Ordinary Reload remains pinned.
   Exercise racing prompts, failure recovery and reconnect reconciliation.
   Preserve workspace, history, drafts, queue, preferences and the auth runtime
   home; never silently substitute a blank native session or account.
5. Any future retirement must prevent only newly authorized selection of an
   exact defective identity while preserving old bytes and captured leases.
   It must not turn ordinary uninstall into a selective-generation migration:
   uninstall affects the whole Machine/Plugin slot and its confirmed sessions.
6. Production installation, each live-session rebind, any active-turn stop,
   retirement-policy activation, and eventual data purge remain separate
   explicit operations. Do not inspect or borrow live credential contents to
   make a fixture pass. Existing sessions continue on their recorded generation
   until their own authorized transition or natural drain.

Remaining work includes a reviewed release/retirement policy, actual native
resume/history evidence, compatible Controller/Machine acceptance and any
authorized production actions. Failure isolation alone completes none of these.
