import { Command } from "./lib/command.ts";
import { tmpdir } from "node:os";
import {
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { test } from "bun:test";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { verifyNativeShell } from "./check-native-shell.ts";

const root = fileURLToPath(new URL("../", import.meta.url));
function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function rejects(run: () => Promise<unknown>, message: string) {
  try {
    await run();
  } catch (error) {
    assert(String(error).includes(message), String(error));
    return;
  }
  throw new Error("expected rejection: " + message);
}
async function copyTree(from: string, to: string) {
  await mkdir(to, { recursive: true });
  for (const entry of await readdir(from, { withFileTypes: true })) {
    if (entry.name === "gen" || entry.name === "target") continue;
    const source = resolve(from, entry.name);
    const target = resolve(to, entry.name);
    if (entry.isDirectory()) await copyTree(source, target);
    else await copyFile(source, target);
  }
}
async function fixture(run: (base: string) => Promise<void>) {
  const base = await mkdtemp(join(tmpdir(), "cowboy-native-source-"));
  try {
    await copyTree(
      resolve(root, "apps/native-shell"),
      resolve(base, "apps/native-shell"),
    );
    await run(base);
  } finally {
    await rm(base, { recursive: true });
  }
}
async function change(
  base: string,
  path: string,
  before: string,
  after: string,
) {
  const file = resolve(base, "apps/native-shell", path);
  const text = await readFile(file, "utf8");
  assert(text.includes(before), "fixture mismatch: " + path);
  await writeFile(file, text.replace(before, after));
}

test("complete native shell is owned by this checkout", () =>
  verifyNativeShell(root));
for (
  const [name, path, before, after, message] of [
    [
      "missing Scene delegate",
      "tauri/Info.ios.plist",
      "<string>TaoSceneDelegate</string>",
      "<string>MissingDelegate</string>",
      "iOS needs a static single-scene",
    ],
    [
      "empty Scene configurations",
      "tauri/Info.ios.plist",
      "<key>UIWindowSceneSessionRoleApplication</key>",
      "<key>WrongSceneRole</key>",
      "iOS needs a static single-scene",
    ],
    [
      "obsolete Scene runtime",
      "tauri/Cargo.lock",
      'name = "tao"\nversion = "0.37.1"',
      'name = "tao"\nversion = "0.35.3"',
      "iOS scene lifecycle requires Tao",
    ],
    [
      "unused native build dependency",
      "tauri/build.rs",
      "tauri_build::build()",
      "",
      "native build must invoke",
    ],
    [
      "disabled native build script",
      "tauri/Cargo.toml",
      "[package]",
      "[package]\nbuild = false",
      "native build must invoke",
    ],
    [
      "unlocked manifest",
      "tauri/Cargo.toml",
      '"=2.12.0"',
      '"2"',
      "unlocked native manifest",
    ],
    [
      "stale lock",
      "tauri/Cargo.lock",
      ' "tauri-plugin-opener",',
      "",
      "root omits dependency",
    ],
    [
      "external Rust source",
      "tauri/Cargo.toml",
      "[dependencies]",
      '[dependencies]\nborrowed = { path = "/tmp/other" }',
      "may not borrow",
    ],
    [
      "external frontend",
      "tauri/tauri.conf.json",
      '"../loader"',
      '"../../../../elsewhere"',
      "frontend escapes",
    ],
    [
      "native file drop interception",
      "tauri/tauri.conf.json",
      '"dragDropEnabled": false',
      '"dragDropEnabled": true',
      "leave file drops to the WebView",
    ],
    [
      "personal team",
      "apple/project.yml",
      "        CODE_SIGN_STYLE: Manual",
      "        DEVELOPMENT_TEAM: ABC123",
      "personal signing team",
    ],
    [
      "unpinned Android NDK",
      "toolchain.json",
      '"ndk": "30.0.16248370"',
      '"ndk": "latest"',
      "exact pins",
    ],
    [
      "Android minSdk drift",
      "tauri/tauri.conf.json",
      '"minSdkVersion": 29',
      '"minSdkVersion": 24',
      "minSdk differs",
    ],
    [
      "template Android activity",
      "android/app/src/main/java/top/thundersparrow/cowboy/MainActivity.kt",
      "class MainActivity : TauriActivity()",
      "class MainActivity : android.app.Activity()",
      "extend the generated TauriActivity",
    ],
    [
      "Android without an authentication browser",
      "android/app/src/main/java/top/thundersparrow/cowboy/MainActivity.kt",
      "authenticationBrowser.install(webView)",
      "",
      "install the authentication browser",
    ],
    [
      "unscoped IPC",
      "tauri/capabilities/remote-haptics.json",
      "https://cowboy.stormbird.xyz/*",
      "https://*/*",
      "IPC must stay scoped",
    ],
  ]
) {
  test("rejects " + name, () =>
    fixture(async (base) => {
      await change(base, path, before, after);
      await rejects(() => verifyNativeShell(base), message);
    }));
}
test("rejects borrowed native symlinks without modifying the source", () =>
  fixture(async (base) => {
    const link = resolve(base, "apps/native-shell/apple/borrowed");
    await symlink(resolve(root, "apps/native-shell/apple/Sources"), link);
    await rejects(() => verifyNativeShell(base), "native source symlink");
    assert((await lstat(link)).isSymbolicLink(), "validator must be read-only");
  }));

test("SSH transport preserves paths and JavaScript without a personal plugin", async () => {
  const base = await mkdtemp(join(tmpdir(), "cowboy-native-ssh-"));
  try {
    const bin = resolve(base, "bin");
    const remote = resolve(base, "worktree ' ; $(printf unsafe)");
    await mkdir(bin);
    await mkdir(resolve(remote, "tools"), { recursive: true });
    // No networking. Execute the actual quoted remote shell body locally.
    await writeFile(
      resolve(bin, "ssh"),
      '#!/usr/bin/env bash\nexec /bin/sh -c "${!#}"\n',
      { mode: 0o755 },
    );
    await writeFile(resolve(bin, "git"), "#!/bin/sh\npwd -P\n", {
      mode: 0o755,
    });
    await writeFile(
      resolve(remote, "tools/cowboysim.sh"),
      '#!/bin/bash\nprintf "%s\\0" "$@"\n',
    );
    const payload =
      "document.querySelector('[data-x=\"a\"]'); $(exit 42)\n'quoted' & |";
    const result = await new Command("bash", {
      args: [resolve(root, "tools/cowboysim-remote.sh"), "eval", payload],
      clearEnv: true,
      env: {
        PATH: bin + ":" + process.env["PATH"],
        COWBOY_SIM_REMOTE_WORKTREE: remote,
      },
      stdout: "piped",
      stderr: "piped",
    }).output();
    assert(result.success, new TextDecoder().decode(result.stderr));
    assert(
      new TextDecoder().decode(result.stdout) === "eval\0" + payload + "\0",
      "SSH changed payload",
    );
  } finally {
    await rm(base, { recursive: true });
  }
});

test("simulator control fails before touching an implicitly selected device", async () => {
  const result = await new Command("bash", {
    args: [resolve(root, "tools/cowboysim.sh"), "launch"],
    clearEnv: true,
    env: { PATH: process.env["PATH"] ?? "" },
    stdout: "piped",
    stderr: "piped",
  }).output();
  assert(
    !result.success &&
      new TextDecoder().decode(result.stderr).includes(
        "select a Simulator explicitly",
      ),
    "implicit device selection",
  );
});

test("native build is locked, fresh and build-only", async () => {
  const builder = await readFile(
    resolve(root, "tools/build-native-shell.sh"),
    "utf8",
  );
  for (
    const contract of [
      "git archive",
      "git status --porcelain",
      "mktemp -d",
      "-- --locked",
      "--no-sign",
      "receipt.json",
      "--receipt-path",
      "ipa_sha256",
      "ditto -c -k --keepParent",
      'open("x")',
    ]
  ) {
    assert(
      builder.includes(contract),
      "missing native build contract: " + contract,
    );
  }
  for (
    const unsafe of [
      "simctl install",
      "simctl launch",
      "DerivedData/",
      "|| echo",
      "cowboy-shell",
      ".codex/",
    ]
  ) {
    assert(
      !builder.includes(unsafe),
      "native build imports unsafe legacy behavior: " + unsafe,
    );
  }
  const swift = await readFile(
    resolve(
      root,
      "apps/native-shell/apple/Sources/cowboy-app/CowboyDevBridge.swift",
    ),
    "utf8",
  );
  for (
    const contract of [
      "#if DEBUG && targetEnvironment(simulator)",
      '["COWBOY_SIM_BRIDGE"] == "1"',
      'host: "127.0.0.1"',
      'fields["origin"] == nil',
      'fields["x-cowboy-simulator"] == self.simulatorID',
    ]
  ) {
    assert(
      swift.includes(contract),
      "missing simulator bridge boundary: " + contract,
    );
  }
});

test("bundled Settings opener uses the actual URL argument and a closed scope", async () => {
  const loader = await readFile(
    resolve(root, "apps/native-shell/loader/index.html"),
    "utf8",
  );
  const start = loader.indexOf("async function openSettings()");
  const end = loader.indexOf('document.getElementById("open-settings")', start);
  assert(start > 0 && end > start, "missing loader Settings action");
  let called = false;
  const run = new Function(
    "globalThis",
    "hint",
    "APP_NAME",
    loader.slice(start, end) + "\nreturn openSettings();",
  );
  await run(
    {
      __TAURI__: {
        core: {
          invoke(command: string, args: Record<string, string>) {
            assert(
              command === "plugin:opener|open_url",
              "wrong opener command",
            );
            assert(
              args.url === "app-settings:" && !("path" in args),
              "wrong opener argument",
            );
            called = true;
            return Promise.resolve();
          },
        },
      },
    },
    { style: {} },
    "Cowboy",
  );
  assert(called, "loader did not invoke Settings opener");
  const local = JSON.parse(
    await readFile(
      resolve(root, "apps/native-shell/tauri/capabilities/default.json"),
      "utf8",
    ),
  );
  assert(
    JSON.stringify(local.permissions[1]) === JSON.stringify({
      identifier: "opener:allow-open-url",
      allow: [{ url: "app-settings:" }],
    }),
    "loader opener scope must be app-settings only",
  );
  const remote = JSON.parse(
    await readFile(
      resolve(root, "apps/native-shell/tauri/capabilities/remote-haptics.json"),
      "utf8",
    ),
  );
  assert(
    remote.permissions.includes("opener:allow-default-urls") &&
      !remote.permissions.includes("opener:default"),
    "web links must not gain file-manager permissions",
  );
});
