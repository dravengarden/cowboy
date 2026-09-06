# Cowboy native shell

The complete Tauri product shell is owned by this Cowboy checkout. It is not a
separate project, a dependency on the old Mac shell directory, or a Codex plugin.
Tauri/Wry, registry crates and Apple's Xcode/SDK are ordinary framework/toolchain
dependencies; the running UI intentionally connects to the Cowboy Controller.
No Controller, Machine, Provider implementation or downloadable plugin UI is
compiled into this thin client.

## Source boundary

- `tauri/`: independent Rust workspace and Cargo.lock, pinned Tauri dependencies,
  capabilities, platform configuration and current Cowboy icons.
- `loader/`: bundled offline-first connection page; no npm or Web build needed.
- `apple/`: handwritten iOS keyboard, clipboard, haptic, Safari and Passkey
  bridges, launch resources, icons and the XcodeGen project specification.
  Generated Xcode projects and Rust outputs are never release inputs.
- `toolchain.json`: explicit Rust, Tauri CLI and XcodeGen versions and provenance.

The shell sources were recovered selectively from this same repository's
`tauri-shell` revision `627e63288fee0cdfdacebfcd04f1180d62f5e356`, not by merging
its old Controller/Web history. Mainline's newer Passkey/native-Plugin ABI and
Cowboy 1703 assets remain authoritative. The predecessor lock omitted opener and
haptics; the independent lock is now reconciled with their exact version pins.
The root Cargo workspace/lock and backend Nix source closure are unchanged.

## Build from clean Git source

On Linux, run `nix develop -c just native-shell-check` from the repository root.
It checks source ownership, closed dependencies, lock pins, native capabilities,
SSH quoting and keyboard geometry. It does not pretend to compile Apple's SDKs.

Apple builds run on an arm64 Mac with Xcode selected and Deno, Python 3, Rustup,
Cargo, `cargo-tauri` and (for iOS) XcodeGen on PATH. Provision the versions in
`toolchain.json` explicitly; the builder never installs missing tools:

```sh
rustup toolchain install 1.97.1 --profile minimal
rustup target add --toolchain 1.97.1 aarch64-apple-ios aarch64-apple-ios-sim
cargo +1.97.1 install tauri-cli --version '=2.11.2' --locked
# Install XcodeGen 2.46.0 through your managed Mac toolchain.
```

Fetch the Cowboy task commit on the Mac and create a session-owned Git worktree.
Do not copy a working source tree over SCP or build from a stable checkout,
personal plugin cache, or an unversioned shell directory. From the clean worktree
root:

```sh
bash tools/build-native-shell.sh macos
bash tools/build-native-shell.sh ios-sim --debug
bash tools/build-native-shell.sh ios
# Equivalent when just is available: just native-shell-build ios-sim --debug
```

Every build creates a new `dist/native-shell/<platform>-<revision>.<nonce>/`,
stages only this Git revision, regenerates the Xcode project from owned source,
uses its own Cargo target directory and builds with `--locked`. The generated
project includes both `CowboyNativeTweaks.mm` and `CowboyPasskeyBridge.mm`.
Compilation/archive failure is fatal. No DerivedData glob, cached static library
or predecessor App is accepted as a substitute. A success receipt identifies the
exact App, Git revision, lock digest, executable digest and actual Xcode version.
These are source-reproducible builds, not a claim of bit-identical signed Apple
bundles across different Xcode/SDK versions.

The entry is **build-only**: macOS is ad-hoc signed; iOS archives are unsigned.
It neither installs/launches an app nor reads signing credentials or changes
provisioning. Distribution signing, device installation, Associated Domains,
real login and physical keyboard/swipe acceptance remain separate release steps.
An unsigned device archive cannot be installed without explicit signing.

## Acceptance

`just native-plugin-conformance` on a Mac compiles both production Objective-C
bridges into an isolated WKWebView fixture and runs ten ABI/coexistence checks on
a new disposable Simulator. This does not replace a full Tauri build or device
acceptance. See [Apple capability policy](apple/README.md) and
[Simulator controls](../../docs/ios-simulator.md).

`just native-shell-smoke <ios-sim build receipt.json>` additionally verifies the
actual Tauri App in its own newly-created Simulator. It signs only a disposable
copy of the exact recorded binary, checks real IPC/native-ABI coexistence and
the eval listener's access boundary, then deletes the Simulator and copy. No
existing App, browser, keychain or device is used; real login is not exercised.

The simulator eval listener is compiled only for Debug Simulator builds and
requires `COWBOY_SIM_BRIDGE=1` at launch. It binds 127.0.0.1, requires the exact
Simulator identity header, rejects browser-origin requests, and exposes no CORS
allowance. Release builds and physical devices contain no eval listener.

No keyboard/composer algorithm was redesigned during consolidation. The known
physical-iPhone pasted-image caret/IME issue remains open; see PITFALLS #69.

Upstream command semantics:
[Tauri CLI](https://v2.tauri.app/reference/cli/).
