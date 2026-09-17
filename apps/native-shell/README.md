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
Returning to Cowboy is never treated as a close, because Chrome can minimize a
Custom Tab into picture-in-picture indistinguishably; the page keeps its
explicit Cancel. Without this bridge the remote UI would navigate the only
WebView to the Provider with no busy state.

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
