# Browser WireGuard experiment

This independent Cargo workspace runs GotaTun 0.9.2 in a browser Web Worker and
in a native Rust fixture server. WSS carries complete encrypted WireGuard packets
between them. Inner traffic consists of valid IPv4/UDP datagrams to one allowed
address/port. No system TUN interface, routes, external VPN or control plane is
used. This code is a research fixture, not a Cowboy runtime component.

`/fixture/*` is trusted test-harness control, including explicit peer installation
and removal. It is **not** a product authentication/enrollment design. The runner
requires a private single-user/network namespace and an otherwise empty network.
It generates disposable keys/certificates and trusts the fixture CA only inside
a disposable Firefox profile. Browser certificate validation stays enabled.

## Build from the repository root

Use the repository's pinned Nix shell for every build/run command. Obtain the
same pinned Rust toolchain with its WASM standard library, a browser and C/NSS
tools (the output-link prefix can be changed):

```sh
nix build --impure --expr '
  let f = builtins.getFlake (toString ./.);
      p = import f.inputs.nixpkgs {
        system = "x86_64-linux";
        overlays = [ f.inputs.rust-overlay.overlays.default ];
      };
  in (p.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml).override {
    targets = [ "wasm32-unknown-unknown" ];
  }' --out-link /tmp/cowboy-wg-rust
nix build .#cowboy-idb-test-browser --out-link /tmp/cowboy-wg-browser
nix build --impure --expr '
  let f = builtins.getFlake (toString ./.);
      p = import f.inputs.nixpkgs { system = "x86_64-linux"; };
  in p.symlinkJoin {
    name = "cowboy-browser-wg-build-tools";
    paths = [ p.llvmPackages.clang-unwrapped p.nss.tools ];
  }' --out-link /tmp/cowboy-wg-build-tools
nix develop -c cargo install wasm-bindgen-cli --version '=0.2.129' --locked \
  --root /tmp/cowboy-wg-bindgen
nix develop -c python3 tools/browser-wireguard-probe/build.py \
  /tmp/cowboy-wg-experiment /tmp/cowboy-wg-rust \
  /tmp/cowboy-wg-bindgen/bin/wasm-bindgen /tmp/cowboy-wg-build-tools/bin/clang
```

`build.py` verifies the published GotaTun source SHA-256, applies the small
`gotatun-wasm.patch` to scratch sources and uses this workspace's Cargo.lock.
The patch supplies browser clocks; native clocks are unchanged. The browser
build selects browser CSPRNG backends and explicitly selects Clang for ring's
WASM C objects. A host GCC setting can pass `cargo check` but produce native
objects that fail to link into WASM; do not omit the target C compiler.

The `.wasm` and generated JavaScript are emitted under the scratch `assets/`
directory. No generated artifact or third-party source is added to the product.
When the clock patch changes, use a fresh scratch directory.

## Run in an isolated network namespace

```sh
nix develop -c unshare --user --map-current-user --keep-caps --net \
  bash -euc 'ip link set lo up; exec python3 -u tools/browser-wireguard-probe/run.py \
    /tmp/cowboy-wg-browser/bin/firefox \
    /tmp/cowboy-wg-build-tools/bin/certutil \
    /tmp/cowboy-wg-experiment/target/debug/wireguard-browser-server \
    /tmp/cowboy-wg-experiment/assets \
    /tmp/cowboy-wg-experiment/result.json'
```

The runner resolves the Firefox symlink to its immutable Nix path. It tears down
its processes, private profile and TLS keys on completion. The result includes
browser version, WASM/native hashes, observed checks and measurement limits.
Expect roughly two to three minutes: the default 120-second WireGuard rekey
threshold is exercised with real clocks, without shortened or mock timers.

The experiment checks a genuine handshake and bidirectional datagrams, server
push, replay in both directions, a modified authentication tag, wrong peer keys,
inner source restrictions, immediate revocation, explicit reauthorization,
carrier reconnect/reset, retransmission after deliberately dropping an initial
handshake, payload integrity and a default-timer session change. Negative checks
require evidence that the offending packet reached the engine, followed by a
successful positive control where appropriate. `Packet::try_into_ipvx` trims
WireGuard padding before the fixture hands an inner UDP payload to its echo
service.

The timer-driven worker runs on a live page. Explicit pause/resume is tested;
physical-device background suspension and page restoration are separate work.
Fixture latency includes JavaScript RPC and polling and runs over loopback, so
it is not a production LAN throughput benchmark. The server accepts only the
fixture UDP service: no Cowboy HTTP, cookie/device authentication, live product
WebSocket, smoltcp TCP stack or browser key persistence is implemented here.

Research and acceptance boundaries: [browser userspace investigation](../../docs/browser-wireguard-transport.md).
