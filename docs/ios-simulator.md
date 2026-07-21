# Cowboy iOS Simulator bridge

Cowboy's Debug iOS shell exposes its WKWebView through a Debug-only loopback
server on Mac port `4171`. The project-owned Mac control surface is
[`tools/cowboysim.sh`](../tools/cowboysim.sh); the generic Codex plugin only
provides the SSH transport.

Install or refresh the helper from the Cowboy checkout on Hawk:

```bash
scp tools/cowboysim.sh macbook-air:/tmp/cowboysim.sh.new
ssh macbook-air 'install -m 0755 /tmp/cowboysim.sh.new "$HOME/cowboy-shell/tools/cowboysim.sh" && rm /tmp/cowboysim.sh.new'
```

From Hawk, invoke it through the repo wrapper, which resolves the installed
`ios-simulator-bridge` plugin and the Mac helper path:

```bash
tools/cowboysim-remote.sh status
tools/cowboysim-remote.sh launch
tools/cowboysim-remote.sh eval document.title
tools/cowboysim-remote.sh shot
```

The Simulator helper expects a Debug Cowboy shell containing
`CowboyDevBridge.swift`; release and physical-device builds must not expose the
listener. `launch` boots and waits for the configured Simulator before starting
the installed app, so it is safe after a Simulator shutdown. Override a changed
default device with `COWBOY_SIM_UDID` on the Mac.

Before accepting evidence, `status` must report all of:

- `CowboyDevBridge: ok`;
- `app origin: tauri://localhost`;
- `document title: Cowboy`;
- an iPhone user agent.

Prefer `eval` with selector-driven DOM actions over pixel taps. Use `shot` only
for visual evidence. The bridge can prove real WKWebView layout and behavior,
but physical-device-only interactions still require device acceptance.
