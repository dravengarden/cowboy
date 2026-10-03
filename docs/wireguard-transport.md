# Self-managed WireGuard transport — Option 2

Research and first interoperability probe, 2026-10-03. Option 1 is deployed;
Option 2 is a proposed product mode with a working isolated transport probe.
No production WireGuard interface, route, firewall rule or peer was installed.
The scope is LAN reachability and a statically configured WireGuard data plane;
Cowboy does not own a VPN control plane. The native probe below does not constrain
PWA clients to an installed VPN: the follow-up [browser userspace research](browser-wireguard-transport.md)
explores WireGuard inside WASM over a direct browser-compatible carrier.

## Rust implementation

GotaTun is the preferred native candidate. Mullvad maintains this Rust userspace
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

## LAN scope and ownership

For native UDP peers, assume each device can reach the server's UDP listener on
the LAN. Browser peers instead need a reachable WSS or WebTransport endpoint on
that same server. Static peer configurations are sufficient: each endpoint
retains its private key and receives the other endpoint's public key through a
trusted channel.
The operator supplies tunnel addresses and narrow AllowedIPs. No discovery,
automatic enrollment, address allocator, NAT hole punching, relay, mesh routing
or central peer inventory is required in Cowboy. Standard `wg`/`wg-quick` can
configure this data plane. [WireGuard configuration](https://www.wireguard.com/quickstart/).

LAN reachability alone does not encrypt traffic. Option 1 remains the HTTPS
application with mandatory device authentication; Option 2 carries that same
application over a WireGuard tunnel, which can use an OS interface or an
in-memory browser network stack. WireGuard peer keys are separate
from the existing P-256/Ed25519 application identities. Removing a VPN peer and
revoking an application session are separate administrative operations.

| Concern | Owner |
| --- | --- |
| LAN reachability, UDP firewall access and stable endpoint | Host/network administration |
| Peer public keys, static tunnel addresses and AllowedIPs | Operator-owned WireGuard profiles |
| Existing system tunnel, routes and peer removal | OS WireGuard tools or an external VPN client |
| Optional native tunnel startup/shutdown | A small local Cowboy network helper, if needed |
| Browser tunnel and direct WSS carrier | Proposed Cowboy WASM worker and server ingress |
| HTTPS, account/device checks and HTTP/WebSocket service | Existing Cowboy application |

## Native/system tunnel choices

**Existing system WireGuard:** the operator brings up the tunnel and points
Cowboy clients at the reachable HTTPS service hostname or IP. Cowboy needs no
WireGuard cryptographic library or new application transport protocol in this
case. This deployment path also works for Browser/PWA with an external VPN
client. It does not meet the separate goal of a PWA that owns its own tunnel.

**Integrated userspace WireGuard:** if Cowboy should bring up the tunnel itself,
GotaTun is the preferred Rust candidate. Accept a local static profile and manage
only its local lifecycle; do not introduce central registration or allocation.
GotaTun supports configuration through standard WireGuard UAPI/tools. Linux
interface creation still needs appropriate network privileges even though the
cryptographic engine runs in userspace. Keep the Controller unprivileged and
place privileged interface operations in a separately supervised helper.
[Upstream configuration and privileges](https://github.com/mullvad/gotatun).

Helper activation is a separate maintenance boundary; Controller/Web releases
must not silently replace the resident Machine or routes. An integrated private
mode should report tunnel failure rather than automatically switching to a
public endpoint.

## Static profile proposal

These are design fields for an optional integrated native client, **not currently
accepted Cowboy options**. Existing system tunnels use their normal WireGuard
configuration instead:

```toml
[wireguard]
endpoint = "192.168.1.20:51820" # hostname or [IPv6]:port also valid
server_public_key = "<operator-supplied pinned WireGuard public key>"
private_key_file = "<device-owned private key file>"
tunnel_address = "10.77.0.10/32"
allowed_ips = ["10.77.0.1/32"]
service_origin = "https://cowboy.example.com"
```

`endpoint` locates the LAN UDP listener; it can be a host or IP. The pinned
public key identifies the peer independently of DNS. `tunnel_address` is the
separate private overlay address. `service_origin` is the HTTPS application
reachable over that overlay, with normal certificate and hostname/IP validation.
A stable service hostname can resolve to the tunnel address while the UDP
endpoint uses the physical LAN address. DNS changes must not change peer trust.

The server's operator-owned profile contains each allowed device's public key
and exact tunnel source address, for example `10.77.0.10/32`. Route only the
Cowboy service address initially; internet forwarding is unnecessary for this
scope. Public-key distribution, address changes and peer removal remain explicit
profile updates through host/network administration.

For Browser/PWA, retain HTTPS and Option 1's account/device checks inside the
tunnel. WebCrypto requires a secure context, and a browser cannot install a
system WireGuard interface. It can run WireGuard in WASM and route its own
application traffic through a userspace stack and a WSS/WebTransport carrier;
that requires application integration rather than OS routes.
[WebCrypto secure context](https://developer.mozilla.org/en-US/docs/Web/API/Crypto/subtle).

An integrated native mobile client still needs the platform VPN lifecycle,
including iOS packet-tunnel extensions and Android VpnService. Compiling a Rust
library does not supply those permissions or lifecycle layers. This applies to
system-wide native VPN integration; a browser-local tunnel has a different
lifecycle and does not require those OS VPN permissions.
[Apple packet tunnels](https://developer.apple.com/documentation/networkextension/nepackettunnelprovider),
[Android VpnService](https://developer.android.com/reference/android/net/VpnService).

## Research conclusion and remaining validation

A static LAN WireGuard data plane is feasible without a Cowboy control plane.
The eight isolated interoperability checks already use this topology and static
peer configuration. They establish Linux transport feasibility, not an integrated
Cowboy server/client release, mobile readiness or production performance.

The PWA goal now takes priority over native helper integration: first validate
an actual browser WASM tunnel through a same-server carrier, then bridge Cowboy's
API and live session protocol into it. The native probe remains useful reference
interop evidence but does not exercise that path. The [browser research](browser-wireguard-transport.md)
records upstream examples, Rust compilation checks, integration boundaries and
the next experiment. Discovery, NAT traversal and a multi-site control plane
remain outside the scope.
