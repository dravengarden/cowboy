# cowboy native shell (Tauri 2)

A **thin native WebView wrapper** around the cowboy UI. It bundles no frontend
and embeds no backend — the window loads the already-https remote UI:

```
https://cowboy.hawk.thundersparrow.top   (caddy → localhost:3333, tailnet-only)
```

## Why this exists

cowboy's backend (axum + ACP bridge + agent supervisor) can only run on hawk —
an iPhone can't spawn coding agents. So the client is fundamentally a *remote
control*. As a pure-web PWA it hit two hard iOS limits that **only a native
WKWebView can fix**:

- the iOS keyboard accessory bar (∧∨ + 完成) can't be removed from web;
- the native file/photo picker collapses the keyboard and blurs the input.

This shell exists purely to put a native WebView around the remote UI so those
become fixable. No Tauri IPC is exposed to the remote origin
(`dangerousRemoteDomainIpcAccess` is intentionally unset).

## Prerequisites (build host = a Mac)

iOS/macOS builds need Xcode — they **cannot** run on hawk (Linux). On the Mac:

- Xcode + command line tools
- `cargo-tauri` (`cargo install tauri-cli --version '^2'`)
- iOS Rust targets: `aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios`
- the device/simulator must be on the **tailnet** to reach the remote URL

## Workflow (code on hawk, build on Mac)

Cross-platform scaffolding (this dir) is authored on hawk and synced to the Mac;
the platform-specific generation + builds happen on the Mac.

```sh
# 0. one-time on the Mac: generate the icon set from cowboy's PWA icon
cargo tauri icon ../web/public/icon-512.png

# 1. one-time: generate the iOS Xcode project (commit gen/apple afterwards)
cargo tauri ios init

# 2. desktop dev (fastest smoke test that the remote URL loads natively)
cargo tauri dev

# 3. iOS dev on a simulator / connected device
cargo tauri ios dev

# 4. release bundles
cargo tauri build              # macOS .app/.dmg
cargo tauri ios build          # iOS .ipa
```

`gen/apple` (and `gen/android` if ever added) are committed; `gen/schemas` and
`target/` are gitignored.

## Native-layer customizations

The point of the native shell is doing what a pure-web PWA can't. What's wired up
(all verified on the iOS 26 simulator):

- **Keyboard accessory bar removed** — `gen/apple/Sources/cowboy-app/CowboyNativeTweaks.mm`
  swizzles the private `WKContentView`'s `inputAccessoryView` to nil (a C
  constructor, runs before any webview). Focusing the composer then shows only
  the standard QuickType bar, not the ∧∨+Done strip.
- **File-picker permissions** — `src-tauri/Info.ios.plist` carries
  `NSCameraUsageDescription` / `NSPhotoLibraryUsageDescription` (+ encryption
  declaration). Tauri **merges** this file into the app Info.plist on every build.
- **Branded launch screen** — `LaunchScreen.storyboard` fills with the
  `LaunchBackground` colour set (light `#f4ecf7` / dark `#15111d`), matching the
  web app's pre-mount splash so cold start has no white flash.

Safe-area insets (`env(safe-area-inset-*)`) are reported correctly by the native
WebView with no extra code — verified with a probe page.

## Gotchas (learned the hard way)

- **Homebrew isn't on the non-interactive PATH.** `tauri ios init`/`build` shells
  out to `xcodegen` / `pod`; over SSH you must `export PATH=/opt/homebrew/bin:$PATH`
  first or it fails trying to `brew install` them.
- **`cargo tauri ios init` regenerates `Info.plist` and the pbxproj from
  templates.** It wipes hand edits to the generated `Info.plist` (→ use
  `Info.ios.plist`, which it merges) but it leaves `LaunchScreen.storyboard`,
  Assets, and your extra `Sources/*.mm` alone. Re-running init is in fact how a
  newly added source file (like `CowboyNativeTweaks.mm`) gets into the build —
  it makes xcodegen re-glob `Sources/`.
- **Simulator software keyboard won't appear** while "Connect Hardware Keyboard"
  is on (`defaults write com.apple.iphonesimulator ConnectHardwareKeyboard -bool false`,
  then reboot the sim) — needed to see/verify the accessory-bar change.
