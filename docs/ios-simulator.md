# Cowboy iOS Simulator controls

The full native shell, build entry and Simulator helper belong to this
repository. No installed Codex plugin or fixed Mac shell directory is needed.
Build from a clean Cowboy Git worktree on the Mac; see
[native shell](../apps/native-shell/README.md). Exchange source revisions through
Git, not SCP.

The Debug Simulator eval bridge is opt-in, binds only 127.0.0.1 (default port
4171), checks the exact Simulator identity and rejects browser-origin requests.
Release and physical-device builds contain no listener. It is local developer
automation, not a public HTTP API.

Select a Simulator explicitly. The helper never chooses another task's booted
device and never installs an app. Once you have separately authorized installing
the exact Debug build into that Simulator:

```sh
export COWBOY_SIM_UDID="<selected Simulator UUID>"
bash tools/cowboysim.sh launch
bash tools/cowboysim.sh status
bash tools/cowboysim.sh eval 'document.title'
bash tools/cowboysim.sh aeval 'return await window.__COWBOY_NATIVE_PLUGIN_HOST.invoke("webauthn", {action:"capabilities",rp_id:"cowboy.stormbird.xyz"})'
bash tools/cowboysim.sh shot /tmp/cowboy-simulator.png
```

`launch` explicitly cold-starts the selected installed app and enables its
bridge using Simulator child environment variables. It does not modify global
Simulator preferences or install a new app. Choose a distinct
`COWBOY_SIM_DEVPORT` for concurrently running tasks.

From Hawk, use Cowboy's direct SSH wrapper. Configure the stable `macbook-air`
SSH alias, or select another trusted alias with `COWBOY_SIM_MAC_HOST`.
The remote path is required and must identify the root of a Cowboy Git worktree:

```sh
export COWBOY_SIM_REMOTE_WORKTREE="/absolute/path/to/session-cowboy-worktree"
export COWBOY_SIM_UDID="<selected Simulator UUID>"
bash tools/cowboysim-remote.sh status
bash tools/cowboysim-remote.sh eval 'document.querySelector("title")?.textContent'
```

The wrapper forwards the selected Simulator/port and quotes each argument
through both SSH and the remote shell. JavaScript with quotes, shell symbols or
newlines stays data. Missing worktree/device selection fails closed. The wrapper
does not resolve personal plugin caches, copy source, or accept unknown SSH keys.

Before accepting evidence, confirm the chosen UUID, bridge response, URL,
native-host version and user agent. The local loader starts at
`tauri://localhost` and then intentionally navigates to
`https://cowboy.stormbird.xyz`; the remote origin alone does not prove the App is
a PWA or a native shell. Check `window.__cowboyNativeShell` and
`window.__COWBOY_NATIVE_PLUGIN_HOST` as well.

For automated full-App acceptance, pass the exact successful Debug Simulator
build receipt. Both commands create and remove their own Simulator and sign only
a disposable copy of that App; they never install over an existing App:

```sh
just native-shell-smoke dist/native-shell/<exact-build>/receipt.json
just native-shell-smoke dist/native-shell/<exact-build>/receipt.json --remote
```

The default checks the actual native App/ABI but may finish on the bundled
loader. `--remote` additionally waits for that loader's own navigation to the
exact Cowboy HTTPS origin and a rendered logged-out sign-in form. It checks
positive remote haptics IPC, rejection of local files and the local-only Settings
URL, plus one credential-free, non-cached `GET /api/auth/status`. It never forces
navigation, submits a form, starts a Passkey/OIDC ceremony, or accesses an account.
Unavailable networking, a stuck loader or missing remote capabilities fail this
gate; a local-loader result is never substituted for remote acceptance.

Each attempt retains a separate `acceptance-<mode>-<revision>.<nonce>/` directory
beside the build receipt, printed at startup. A passing attempt writes
`smoke-receipt.json`; a failed attempt writes `failure.json` and, when available,
an App-only Simulator log. This prevents an older successful receipt from being
mistaken for the result of a failed rerun. Neither mode establishes signed
distribution, real-login, physical-device or keyboard/swipe acceptance.

The isolated `just native-plugin-conformance` fixture creates and removes its
own Simulator and app. It does not connect to a real login or validate a full
Tauri bundle. Layout, keyboard, gesture and physical-device acceptance must not
be inferred from that fixture.

The physical-iPhone image-adjacent caret issue remains open (PITFALLS #69).
Simulator HID Return can update CM6 and draw a CSS caret on an empty line whose
Range height is zero; an iPhone may leave its UIKit caret at the prior measurable
text. A Simulator screenshot is not acceptance for that bug.
