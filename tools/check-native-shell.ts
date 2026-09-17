import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

function requireValue(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

// Read-only: never repair, unlink or copy a borrowed native project.
export async function verifyNativeShell(repository: string): Promise<void> {
  const root = await Deno.realPath(repository);
  const native = resolve(root, "apps/native-shell");
  requireValue(
    await Deno.realPath(native) === native,
    "native shell must be owned by this checkout, not a source symlink",
  );
  async function walk(path: string): Promise<void> {
    for await (const entry of Deno.readDir(path)) {
      requireValue(!entry.isSymlink, "native source symlink: " + entry.name);
      if (entry.isDirectory) {
        // Cargo/Tauri outputs are never release inputs (Git archive stages only
        // tracked sources); refuse even output symlinks before skipping them.
        if (
          path === resolve(native, "tauri") &&
          ["target", "gen"].includes(entry.name)
        ) continue;
        await walk(resolve(path, entry.name));
      }
    }
  }
  await walk(native);
  const read = (path: string) => Deno.readTextFile(resolve(native, path));
  const toolchain = JSON.parse(await read("toolchain.json"));
  const manifest = await read("tauri/Cargo.toml");
  const lock = await read("tauri/Cargo.lock");
  // The pinned unused-dependency checker cannot scan build.rs. Keep its one
  // documented build-dependency exception backed by the real closed entrypoint.
  requireValue(
    !/^\s*build\s*=/m.test(manifest) &&
      /^\s*fn\s+main\(\)\s*\{\s*tauri_build::build\(\);?\s*\}\s*$/.test(
        await read("tauri/build.rs"),
      ),
    "native build must invoke tauri_build::build from the default build.rs",
  );
  requireValue(
    /^\[workspace\]$/m.test(manifest),
    "native crate needs an independent workspace",
  );
  requireValue(
    !/\b(?:path|git|workspace)\s*=/.test(manifest),
    "native crate may not borrow path/git/workspace dependencies",
  );
  requireValue(
    manifest.includes('rust-version = "' + toolchain.rust + '"'),
    "Rust toolchain pin differs",
  );
  const names = [
    "tauri",
    "tauri-build",
    "tauri-plugin-opener",
    "tauri-plugin-haptics",
  ];
  requireValue(
    JSON.stringify(Object.keys(toolchain.crates).sort()) ===
      JSON.stringify(names.sort()),
    "unexpected native dependency inventory",
  );
  const packages = lock.split("[[package]]").slice(1);
  for (const [name, version] of Object.entries(toolchain.crates)) {
    requireValue(
      typeof version === "string" && /^\d+\.\d+\.\d+$/.test(version),
      "native dependencies need exact versions",
    );
    const declaration = manifest.split("\n").find((line) =>
      line.startsWith(name + " = ")
    );
    requireValue(
      declaration?.includes('"=' + version + '"'),
      "unlocked native manifest dependency: " + name,
    );
    requireValue(
      packages.some((block) =>
        block.includes('name = "' + name + '"\n') &&
        block.includes('version = "' + version + '"\n')
      ),
      "native lock missing pinned crate: " + name,
    );
  }
  const app = packages.find((block) => block.includes('name = "cowboy-app"\n'));
  requireValue(app, "native lock is missing cowboy-app");
  for (const name of names) {
    requireValue(
      app.includes(' "' + name + '"'),
      "native lock root omits dependency: " + name,
    );
  }
  for (const block of packages) {
    if (block === app) continue;
    requireValue(
      block.includes(
        'source = "registry+https://github.com/rust-lang/crates.io-index"',
      ) &&
        /^checksum = "[a-f0-9]{64}"$/m.test(block),
      "native lock must use checksummed registry dependencies",
    );
  }
  const config = JSON.parse(await read("tauri/tauri.conf.json"));
  const frontend = resolve(native, "tauri", config.build.frontendDist);
  const local = relative(native, frontend);
  requireValue(
    !isAbsolute(local) && local !== ".." && !local.startsWith(".." + sep),
    "native frontend escapes checkout",
  );
  requireValue(
    frontend === resolve(native, "loader"),
    "native shell must embed its owned loader",
  );
  requireValue(
    config.identifier === "top.thundersparrow.cowboy",
    "native identity changed",
  );
  // Tauri's own drag-drop handler swallows OS file drops before the WebView
  // sees them, which would disable the Composer's HTML5 file drop.
  requireValue(
    config.app.windows.every(
      (window: { dragDropEnabled?: boolean }) =>
        window.dragDropEnabled === false,
    ),
    "native windows must leave file drops to the WebView",
  );
  requireValue(
    !config.build.beforeBuildCommand && !config.build.beforeDevCommand,
    "native build may not invoke an ambient frontend",
  );
  for (const path of config.bundle.icon) {
    requireValue(
      !isAbsolute(path) && !path.split(/[\\/]/).includes(".."),
      "external native icon path",
    );
    await Deno.stat(resolve(native, "tauri", path));
  }
  for (
    const path of [
      "loader/index.html",
      "tauri/build.rs",
      "tauri/src/lib.rs",
      "tauri/src/main.rs",
      "tauri/Info.ios.plist",
      "apple/project.yml",
      "apple/LaunchScreen.storyboard",
      "apple/Sources/cowboy-app/main.mm",
      "apple/Sources/cowboy-app/bindings/bindings.h",
      "apple/Sources/cowboy-app/CowboyNativeTweaks.mm",
      "apple/Sources/cowboy-app/CowboyPasskeyBridge.mm",
      "apple/Sources/cowboy-app/CowboyAppIconBridge.mm",
      "apple/Sources/cowboy-app/CowboyKeyboardGeometry.h",
      "apple/Sources/cowboy-app/CowboyDevBridge.swift",
      "apple/Assets.xcassets/AppIcon.appiconset/Contents.json",
      "android/app/src/main/java/top/thundersparrow/cowboy/MainActivity.kt",
      "android/app/src/main/java/top/thundersparrow/cowboy/AuthenticationBrowser.kt",
    ]
  ) await read(path);
  // Android: SDK Manager-owned components are pinned exactly, the Tauri
  // config agrees with the pin, and the generated activity is replaced by the
  // owned Kotlin source rather than Tauri's template.
  const android = toolchain.android;
  requireValue(
    typeof android?.ndk === "string" &&
      /^\d+\.\d+\.\d+$/.test(android.ndk) &&
      /^android-\d+$/.test(android.platform) &&
      /^\d+\.\d+\.\d+$/.test(android.buildTools),
    "Android SDK components need exact pins",
  );
  requireValue(
    config.bundle.android?.minSdkVersion === android.minSdk,
    "Android minSdk differs from the toolchain pin",
  );
  requireValue(
    JSON.stringify(
      (android.abis as {
        name: string;
        tauriTarget: string;
        rustTarget: string;
      }[])
        .map((abi) => [abi.name, abi.tauriTarget, abi.rustTarget]),
    ) === JSON.stringify([
      ["arm64-v8a", "aarch64", "aarch64-linux-android"],
      ["x86_64", "x86_64", "x86_64-linux-android"],
    ]),
    "unexpected Android ABI inventory",
  );
  const androidActivity = await read(
    "android/app/src/main/java/top/thundersparrow/cowboy/MainActivity.kt",
  );
  requireValue(
    /class MainActivity : TauriActivity\(\)/.test(androidActivity),
    "Android activity must extend the generated TauriActivity",
  );
  // Without the authentication browser the remote UI navigates the only
  // WebView to the Provider, where repeated taps exhaust OIDC transactions.
  requireValue(
    androidActivity.includes("authenticationBrowser.install(webView)") &&
      androidActivity.includes("authenticationBrowser.onResume()"),
    "Android activity must install the authentication browser",
  );
  const project = await read("apple/project.yml");
  requireValue(
    !/DEVELOPMENT_TEAM:/.test(project),
    "default project must not require a personal signing team",
  );
  requireValue(
    project.includes("path: Sources"),
    "Xcode must include both owned bridges",
  );
  const capabilities = JSON.parse(
    await read("tauri/capabilities/remote-haptics.json"),
  );
  requireValue(
    JSON.stringify(capabilities.remote.urls) === JSON.stringify([
      "https://cowboy.stormbird.xyz",
      "https://cowboy.stormbird.xyz/*",
    ]),
    "remote native IPC must stay scoped to Cowboy",
  );
}

if (import.meta.main) {
  await verifyNativeShell(dirname(dirname(fileURLToPath(import.meta.url))));
  console.log("Native shell source and lock boundary verified");
}
