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
