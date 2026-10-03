# Self-managed WireGuard transport — Option 2

Research and first interoperability probe, 2026-10-03. Option 1 is deployed;
Option 2 is a proposed product mode with a working isolated transport probe.
No production WireGuard interface, route, firewall rule or peer was installed.

## Rust implementation

GotaTun is the preferred candidate. Mullvad maintains this Rust userspace
implementation, derived from BoringTun, with a library and a standalone daemon.
Its documented targets include Linux, macOS and Windows, plus mobile library
targets. The repository uses MPL-2.0 for current contributions; retain its
license and attribution when packaging. [Upstream repository](https://github.com/mullvad/gotatun).

Pin **0.9.2** for the initial investigation. Cowboy's pinned Rust 1.98.1 builds
it successfully; upstream requires Rust 1.95 or later. Do not choose an older
version merely because it predates API changes: subsequent releases include
nonce-limit, UDP receive, AllowedIPs, malformed-key and packet-validation fixes.
[Upstream changelog](https://github.com/mullvad/gotatun/blob/v0.9.2/CHANGELOG.md).

Assured's independent review covered GotaTun **0.2.0**, excluding the CLI,
DAITA and external dependencies. Mullvad reports two low-severity findings and
their fixes. That review is useful evidence, not an audit certificate for the
entire 0.9.2 integration. [Audit scope and response](https://mullvad.net/en/blog/a-security-audit-of-gotatun-is-now-available).

Cloudflare's BoringTun is a credible alternative with deployed users, but its
README warns against depending on its restructuring master branch.
DefGuard's wireguard-rs is a management abstraction over existing implementations,
not another Rust cryptographic engine. [BoringTun](https://github.com/cloudflare/boringtun),
[DefGuard wireguard-rs](https://github.com/DefGuard/wireguard-rs).

## Evidence from this checkout

The [probe](../tools/wireguard-transport-probe.py) runs two separate network
namespaces inside a private unprivileged user/mount/network namespace. GotaTun
0.9.2 serves one peer; the reference wireguard-go implementation serves the
other. Keys, certificates, hosts entries, UAPI sockets and interfaces are
disposable. No production credentials are used.

The [receipt](experiments/wireguard-transport-2026-10-03.json) records eight
passing checks: IP and hostname endpoints, HTTPS with certificate verification,
rejection of a wrong TLS hostname, rejection of an unauthorized inner source
address, rejection of an incorrect server public key at the correct endpoint,
recovery with the correct key, and revocation of an established device peer.

Reproduce from the repository root in the pinned shell:

```sh
nix develop -c cargo install gotatun-cli --version '=0.9.2' --locked \
  --root /tmp/cowboy-option2-gotatun
nix develop -c unshare --user --map-root-user --mount --net --fork \
  python3 tools/wireguard-transport-probe.py \
  /tmp/cowboy-option2-gotatun/bin/gotatun \
  /absolute/network-tools/bin /absolute/receipt.json
```

The tools directory must contain `wg`, `wireguard-go`, `ip` and `ping`; the
receipt pins the Nix tools closure used here. The script refuses an ordinary
host user namespace or a populated network namespace.

## Proposed configuration and integration

These are design fields, **not currently accepted Cowboy options**:

```toml
transport = "wireguard" # alternative: "https" (Option 1)

[wireguard]
endpoint = "gateway.example.com:51820" # IPv4 or [IPv6]:port also valid
server_public_key = "<pinned WireGuard public key>"
private_key_file = "<device-owned private key file>"
tunnel_address = "10.77.0.10/32"
allowed_ips = ["10.77.0.1/32"]
service_origin = "https://cowboy.example.com"
```

`endpoint` locates the public UDP listener. The pinned public key identifies the
server, independently of DNS or its current IP. `tunnel_address` is the private
overlay address. `service_origin` names the HTTPS application inside that
overlay. Endpoint DNS refresh and roaming must retain the same pinned identity.
Changing a DNS record must never enroll a different server key.

The initial topology should be a reachable server hub and outbound device
peers, with only the Cowboy service prefix routed through the tunnel. Each
device generates a separate WireGuard key locally; the server stores its public
key and exact assigned address. Keep WireGuard keys distinct from the existing
P-256/Ed25519 application identities. Use authenticated enrollment to deliver the
server key and register the device key; never infer trust from a first UDP
response. Device revocation removes both its application authority and its VPN
peer. Persist allocation and revocation before acknowledging either operation.

Keep the Controller unprivileged. A separately supervised network helper should
own TUN creation, bounded route/firewall updates, key files and peer lifecycle.
The existing HTTPS proxy and HTTP/WebSocket application can then operate over
the private interface. Network-helper deployment is its own maintenance boundary;
Controller/Web releases must not silently replace the resident Machine or routes.
Tunnel failure must stay visible rather than falling back to public transport.

For Browser/PWA, retain HTTPS and Option 1's account/device checks inside the
tunnel. WebCrypto requires a secure context, and a browser cannot install a
system WireGuard interface. The two product modes therefore mean public HTTPS
or a private WireGuard network carrying the same HTTPS service. An IP endpoint
for WireGuard does not require an IP-based HTTPS origin; using a stable service
hostname keeps certificate validation straightforward. [WebCrypto secure context](https://developer.mozilla.org/en-US/docs/Web/API/Crypto/subtle).

Desktop clients need a network helper or an installed WireGuard client. Native
mobile integration needs the platform VPN lifecycle, including iOS packet-tunnel
extensions and Android VpnService; compiling a Rust library does not provide
those lifecycle/permission layers. A configured external VPN client can supply
the tunnel to the current PWA first. [Apple packet tunnels](https://developer.apple.com/documentation/networkextension/nepackettunnelprovider),
[Android VpnService](https://developer.android.com/reference/android/net/VpnService).

## Work remaining before product activation

Implement explicit mode validation, authenticated pairing and revocation,
durable peer/address inventory, the isolated helper's IPC contract, platform
install/uninstall and recovery receipts, route conflict detection and private
service exposure. Then test reconnects, DNS/address changes, UDP loss/blocking,
MTU and large artifact transfers, sleep/wake, device revocation during live
WebSockets, and helper/server restarts. UDP reachability still matters: WireGuard
does not supply a relay or general NAT traversal control plane. Persistent
keepalive can maintain an existing NAT mapping. [WireGuard configuration](https://www.wireguard.com/quickstart/).

The current probe establishes Linux interoperability and feasibility. It does
not establish mobile readiness, production performance, a complete enrollment
protocol, or a self-healing multi-site VPN network.
