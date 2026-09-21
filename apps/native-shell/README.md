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
- `android/`: handwritten Kotlin sources overlaid onto Tauri's generated Gradle
  project. Generated Gradle projects and Rust outputs are never release inputs.
- `toolchain.json`: explicit Rust, Tauri CLI, XcodeGen and Android SDK/NDK
  versions and provenance.

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
rustup component add llvm-tools --toolchain 1.97.1-aarch64-apple-darwin
cargo +1.97.1 install tauri-cli --version '=2.11.2' --locked
# Install XcodeGen 2.46.0 through your managed Mac toolchain.
```

swift-rs calls the `llvm-tools` component's `llvm-objcopy`; Apple ships no
equivalent. `build-native-shell.sh` checks for it before every device build
because its absence degrades to a `cargo:warning` that cargo hides for registry
dependencies, and only surfaces ten minutes later as undefined symbols.

## Blocked: iOS device builds need Xcode 26.x (2026-09-21)

**Do not report an iOS release as shippable until this clears.** The last
successful one is 0.1.31 (revision `ed8bad98`, 2026-09-15) built under Xcode
26.6 (17F113). The Mac's Xcode was then upgraded in place to 27.0 (27A266a),
leaving no 26.x to `xcode-select`, and `bash tools/build-native-shell.sh ios`
has not produced an IPA since.

Three failure layers, peeled in order. The first two are fixed and committed:

1. swift-rs 1.0.7 fed swiftc an iOS `-sdk`/`-target` pair followed by a macOS
   pair that overrode it, so iOS Swift sources compiled against MacOSX27.0.sdk
   and died on `CIContext.h: 'OpenGLES/EAGL.h' file not found`. swift-rs 1.0.8
   is the release carrying the Xcode 27 fixes; the lock now pins it.
2. Xcode 27's SwiftPM internalizes `@_cdecl` exports in static products.
   swift-rs 1.0.8 promotes them back with `llvm-objcopy`, which needs the
   `llvm-tools` component above.
3. **Open.** swift-rs 1.0.8 promotes only symbols from a package's own object
   member, so its own Swift runtime exports stay local:

   ```text
   $ nm libTauri.a
   000000000000b744 T _register_plugin     # Tauri.o    — promoted
   0000000000000d38 t _release_object      # SwiftRs.o  — still local
   0000000000000d30 t _retain_object       # SwiftRs.o  — still local
   0000000000000e3c t _string_from_bytes   # SwiftRs.o  — still local
   ```

   The link then fails on exactly those three. The guard is deliberate:
   `SwiftRs.o` is embedded in `libTauri.a`, `libtauri-plugin-haptics.a` and
   `libtauri-plugin-opener.a` alike, and promoting it in each produces duplicate
   globals that crash Xcode 27's `ld` with "malformed atom files with duplicate
   names" — `-ld_classic` is gone, so there is no escape hatch. A correct fix
   promotes the runtime in exactly one archive, which is upstream's call, not a
   local patch. As of 2026-09-21 `Brendonovich/swift-rs` has no commit after the
   1.0.8 release.

Do not work around this by pointing `[patch.crates-io]` at a fork:
`tools/check-native-shell.ts` requires every locked native package to carry a
`registry+` source and a checksum, and a fork would violate that supply-chain
policy rather than satisfy it. Changing the policy is a separate, explicit
decision.

Unblock when upstream ships the runtime-globalization fix (then bump the lock
and rebuild), or when a Mac with Xcode 26.x is available (then the current lock
already builds — swift-rs 1.0.8's objcopy path is Xcode-27-only).

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

Device builds also package that exact App as `Cowboy.ipa` and record its path
and SHA-256 in the build receipt. A machine-owned publisher may request an
exclusive copy with `--receipt-path /absolute/new.json`; an existing file is
never overwritten. The SideStore workflow fetches the clean public Cowboy
revision through Git into a fresh Mac task checkout, calls this entry, verifies
the receipt and IPA digest on Hawk, and then publishes through its normal
versioned source. The old external Mac shell is not a source or artifact fallback.

## Android

Android builds run on a Linux host (Hawk) inside the pinned shell:

```sh
nix develop .#native-android -c bash -c \
  'ANDROID_HOME=$HOME/Android/Sdk bash tools/build-native-shell.sh android-emu --debug'
nix develop .#native-android -c bash -c \
  'ANDROID_HOME=$HOME/Android/Sdk bash tools/build-native-shell.sh android'
```

The shell supplies rustc and the Android Rust targets plus the exact Tauri
CLI, all read from `toolchain.json`. The SDK, NDK, platform and build-tools are
owned by Android Studio's SDK Manager; the builder verifies their pinned
versions and never installs them. `android-emu` builds an x86_64 APK for the
Android Emulator; `android` builds arm64-v8a for physical devices.

Like the Apple entry, each build stages only this Git revision, regenerates
the Gradle project with `cargo tauri android init`, overlays `android/` and
records a receipt with the APK digest, application id, SDK levels, ABI,
Gradle, Android Gradle Plugin, NDK and signing state. Debug APKs carry the
local Android debug keystore; release APKs are unsigned. Installation, real
login and physical-device acceptance remain separate steps.

Sideload signing is its own step and accepts only an exact release receipt:

```sh
nix develop .#native-android -c bash -c '
  export ANDROID_HOME=$HOME/Android/Sdk
  export COWBOY_ANDROID_KEYSTORE=$HOME/.local/share/cowboy-android-signing/release.jks
  export COWBOY_ANDROID_KEYSTORE_PASSWORD_FILE=$HOME/.local/share/cowboy-android-signing/release.password
  bash tools/sign-android-apk.sh /absolute/dist/native-shell/android-<rev>.<nonce>/receipt.json'
```

It re-verifies the APK digest, 16 KB page-aligns, signs (apksigner selects APK
Signature Scheme v3 for minSdk 29), verifies the result and writes `signing-receipt.json` with the
signed APK digest and certificate SHA-256. The keystore and password file stay
outside every checkout (Hawk: `~/.local/share/cowboy-android-signing/`, mode
0700). Every future update must be signed with the same key; back it up.

`android/app/src/main/java/top/thundersparrow/cowboy/MainActivity.kt` keeps
Tauri's edge-to-edge window but applies system bar, display cutout and IME
insets to the WebView container. Android 15+ forces edge-to-edge, and the
remote UI has no Android safe-area contract, so without this the page renders
under the status bar and the keyboard covers the Composer. Emulator smoke
checks run on Hawk with the SDK emulator through `android-fhs`.

`AuthenticationBrowser.kt` installs the iOS shell's page contract for Provider
sign-in (`__cowboyOpenAuthenticationBrowser`, bridge version 2) through an
origin-scoped `WebMessageListener`. Cardea and Provider pages open in a Custom
Tab inside Cowboy's task while the WebView keeps waiting on the PKCE-bound
handoff; completion relaunches the `singleTask` activity to dismiss the tab.
The tab is a partial bottom sheet so Cowboy stays visible: behind a full-screen
tab Android's cached-app freezer stops the app and its WebView renderer, and
the page never sees the ready handoff. For the same reason the WebView, which
Tauri pauses with the activity, is resumed while the sheet covers a visible
Cowboy. Only the tab finishing (its activity
result) is a user close; minimizing the tab is not. Without this bridge the
remote UI would navigate the only WebView to the Provider with no busy state.

`NativeHaptics.kt` gives the remote UI `__cowboyNativeHaptic(kind)` and maps
selection, impact and notification haptics to `View.performHapticFeedback`
(tick, click, confirm and reject effects that follow the system touch-feedback
setting). The Tauri haptics plugin's Android side plays 40-60 ms raw vibrator
waveforms per tap, which feels like a buzzing motor; it remains only as the
fallback for older page bundles.

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
