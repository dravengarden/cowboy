# Machine browser lifecycle fixes (2026-11-24)

Status: released from `d05e5b109` to the Hawk Web and Controller and to the
OVH Machine host. Not yet exercised with a live browser on OVH (the
existing accounts are not opened for tests).

## Change

Found by reading the Machine browser against two weeks of OVH and Controller
records, which show no failed open, launcher error or confinement finding
(four opens in the retained OVH window, ready in 211 to 504 ms; every
`viewer_error` matches the phone leaving the app, reattached within 1 to 3 s
of its return).

| Defect | Now |
| --- | --- |
| A full Machine (`browser.max_instances = 1` on OVH) refused another account's open with `capacity` while its only instance was one nobody viewed, for the 120 s that instance stays after its viewer left | The open hibernates instances detached for 5 s, longest first, with their stop-time snapshot; one being started or viewed is never taken, and nothing is hibernated when that could not free a slot |
| The runtime cap killed the browser with no stop-time copy: every sign-in since the last periodic snapshot (up to 10 min) was lost | A graceful stop begins 30 s ahead; its launcher exchange ends at the cap, which still only kills |
| `deadline_ms` of a detached instance was the runtime cap, shown as "Hibernates in 1790 s unless attached" | A running state reports when the Machine stops it if nothing changes: the end of the grace while detached, else the graceful stop ahead of the cap |
| The Controller cached its own detach's `superseded` answer on a running state, shown as "A newer instance replaced this one" | A reason on a running state is the answering command's outcome and is not cached |
| Why a browser stopped, and whether unsaved state was lost, vanished at the next state read (the Machine answered only that nothing runs) | The Machine keeps how each account's last instance stopped and `BrowserQuery` answers with it |
| An open panel reopened the browser after any stop that ended its stream, the runtime cap included | A reconnect reopens only on its Machine's word that the browser hibernated for being left (`grace`, or no reason known) |
| Supervision awaited a running snapshot copy (and the lifecycle lock) before a grace stop, during which a viewer attached meanwhile got no lease expiry | Both stops are put off to the next tick instead |
| Retry on a refused open disappeared at the next state read (5 s) | The open's refusal is kept apart from the observation and lasts as long as its banner |

Also: a capacity refusal is logged (`browser.open.refused`), the Controller
closes the Machine's half of a display stream with a handshake so the Machine
records `viewer_closed` rather than `failed`, an attached viewer is told two
minutes ahead of the cap, and `max_instances = 0` answers `not_available`.

The design text is in `docs/machine-browser.md` (Lifecycle, Stream channel,
Viewer).

## Review

Managed review `browser-audit-20261124` (Sol Max), three rounds: 12, 9 and 9
findings. Declined with reasons in the commit messages: separate negotiated
stop-time fields (one display consumer; a second field on a
`deny_unknown_fields` event needs protocol gating), an immediate kill at the
cap and an independent termination armed there (the cap has always begun the
unit's ordinary termination, and `RuntimeMaxSec` backs it), and refusing an
open of an already running browser at `max_instances = 0`. The fixes for the
third round were gated but not reviewed again: that review had no round left.

## Gates

| Gate | Result |
| --- | --- |
| `just check-compact` on `d05e5b109` | Passed |
| Machine browser, Controller browser and memory reclaim tests | 210 passed (17 new in `manager/tests/room_and_deadline.rs`) |
| Web `machineBrowser` tests, typecheck, lint, structure | 155 passed |

## Activation

| Target | Result |
| --- | --- |
| Hawk Web | `/nix/store/b0ydmqbwgs2fbv70zqc5y4m5yl3x33zn-cowboy-web-release`, receipt `succeeded` |
| Hawk Controller | `/nix/store/m464k8q34hlvw5x8acj992nl6c95a9i3-cowboy-controller-release`, receipt `succeeded`; `/healthz` ok, four Machines reconnected |
| OVH Machine host | `tools/ovh-machine-host-release.sh` with `/nix/store/7rb8xlizdp63zhjwqj1qdq8f508ca6y2-cowboy-machine-release` (previous `4516gdmh…`, `53deef62`); receipt `/var/lib/columbus/ovh-machine-d05e5b109ece-20261124T031527Z`, 18 workers retained, accepted |

The OVH host release also carries the Machine-side commits on `main` since
`53deef62` that are not part of this change (`4c2cd8538`, `44b2601d7`,
`a140323ef`, `82a7db212`, `9bd968b33`).

After it, OVH reports 17 active sessions and the browser capability, and
every enabled account's browser reads `hibernated`. Previous Hawk releases,
for a rollback: Web `8rw2ar5a…`, Controller `cfhjj3hb…` (both `4285e8876`).
