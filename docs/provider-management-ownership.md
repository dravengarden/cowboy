# Core-owned Provider sign-in and confirmation

This eighteenth spatiotemporal slice extends the
[interactive Provider surface owner](provider-ui-ownership.md) through the core
Service sign-in and Machine uninstall-confirmation dialogs. It is not a new
Plugin capability, runtime ABI, credential authority, or distributed executor.

## Explicit local lifetimes

`providerDialogOwner.ts` gives each open dialog a fresh object-identity lease,
immutable observation snapshot and owned resource scope. Starting a write
reserves its task and busy state synchronously, before subscribers or callbacks
can reenter. A retired lease cannot bind responses, publish errors, clear input,
close the replacement dialog or clear its busy indicator. Provider ID, request
ID and an equal Catalog object alone never identify a view incarnation.

The React hook creates owners at layout-effect commit. Cleanup/replay creates a
fresh owner; speculative rendering does not perform lifecycle work. Machine
panels retain their Machine-ID keys. Submitted HTTP writes do not receive the
view's abort signal. Disposal seals observation immediately and waits for actual
settlement; stuck writes remain draining. Each panel admits at most 16 live or
draining dialog leases per domain, rather than accumulating unbounded retired
requests. A cleanup failure remains in the scope's `needs_reconcile` accounting;
React cleanup cannot certify remote recovery.

## Service authentication

`providerAuthenticationOwner.ts` owns method selection, start, challenge
observation, input submission, explicit cancellation and clipboard feedback.
The selected executor still comes from the current verified Catalog projection,
and the existing start request carries its exact release version and digest.
The Controller independently authorizes and validates every request.

One lease admits one mutation, one pending status read, one clipboard task and
one Catalog refresh at a time. Status reads have an eight-second abort deadline;
the next poll is scheduled 750ms **after** settlement, not by an overlapping
interval. A failed cancellation may resume observation but must still wait for
an old aborted read/body to settle. Replacing or closing the view stops timers
and aborts read-only fetches; transports that ignore abort remain honestly
pending. The optional native browser is never closed by an unmount finalizer.

Start receipts validate request ID/expiry together. Status decoding validates
the response and each event's request/Provider identity and typed fields before
projecting them. Extra API fields confer no UI authority. Missing/expired
requests return the current dialog to method selection; authorization or invalid
status responses stop polling with a visible error. Late response-body reads
repeat the incarnation check. Transient read failures may retry; mutations are
never automatically retried.

Submitting an input reserves admission before React can render the disabled
button; failed submission stays visible and retains the current input. A newer
durable Service success outranks a late submit rejection. Clipboard completion
cannot populate a replaced request or expose an old device code in a new view.
These ephemeral inputs are never persisted or logged by the owner.

`Close`, backdrop dismissal and unmount end observation, not the submitted
Service sign-in. `Cancel sign-in` and `Back` are separate explicit DELETE
requests for the current request, disabled during mutation/promotion. Failed
cancellation does not silently return to methods or erase the request; a
404/410 only establishes that the Service request is already absent. Neither
HTTP acceptance nor browser closure proves that credentials, a remote helper or
other external effects were reverted. Normal expiry and Service authority still
govern detached requests. The native bridge ABI and cross-domain native-browser
arbitration are unchanged; this is not physical-device/login acceptance.

## Machine uninstall confirmation

`providerUninstallOwner.ts` owns latest-preview observation and its exact
Machine/Plugin/plan snapshot. It rejects mismatched targets, malformed impact
sets and invalid deadlines before offering confirmation. Response objects are
projected/copied and frozen so consent cannot be retargeted through mutation.
Confirmation rechecks expiry and explicit active-session consent synchronously,
not only through a disabled JSX button. It submits one request with the captured
plan ID and acknowledgement. The Service's actor/impact/expiry checks and durable
operation journal remain authoritative.

Closing a pending confirmation does not cancel uninstall. An old success cannot
close a newer preview, and an old failure cannot overwrite its current error.
Current failures appear inside the confirmation sheet rather than behind it.
No automatic retry, undo request, worker restart, data deletion or compensation
is introduced by this view owner.

## Verification and release boundary

The complete gate includes typed owner tests for reentrancy, immutable consent,
bounded pending work, failed cleanup, late start/body/submit/cancel/clipboard
responses, poll deadlines, malformed evidence, expiry and confirmation races.
`just provider-management-browser-conformance /nix/store/…/bin/firefox` drives
the production committed-owner hook with real React development StrictMode/MUI
controls and fake deferred HTTP effects in a fresh loopback-only browser
profile. It is a hook/owner integration harness, not an authenticated test of
the full production modal or a native shell. Existing Provider UI and IndexedDB
browser suites remain independent.

Only core Web code and tests change. Public component versions, Plugin
source/binding closures, signed Catalog bytes, Controller/Machine protocols,
credentials and native binaries are unchanged. Release through the Web component
activator and verify that Controller, Machine and worker identities stay intact.

P1 still needs accepted CoreSecurity production policy and public legacy
SDK/native retirement. P2 still needs Victoria's dual-authority durable binding
activation/revocation and recovery; closing a view supplies none of that
authority. Other management actions (for example fallback install/logout rows),
cross-panel/native arbitration and distributed compensation are not claimed to
be covered by these dialog owners.
