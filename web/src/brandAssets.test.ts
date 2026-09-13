import { assert, assertEquals } from "jsr:@std/assert";

const repoRoot = new URL("../../", import.meta.url);
const read = (path: string) => Deno.readTextFile(new URL(path, repoRoot));
const bytes = (path: string) => Deno.readFile(new URL(path, repoRoot));

async function pngSize(path: string): Promise<[number, number]> {
  const data = await bytes(path);
  assertEquals([...data.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  return [view.getUint32(16), view.getUint32(20)];
}

Deno.test("default 103 exports cover web, native, Manager and website", async () => {
  for (
    const [path, size] of Object.entries({
      "assets/brand/cowboy-logo.png": 1024,
      "web/public/cowboy-app-icon-180-v6.png": 180,
      "web/public/cowboy-app-icon-192-v6.png": 192,
      "web/public/cowboy-app-icon-512-v6.png": 512,
      "web/public/cowboy-app-icon-maskable-512-v6.png": 512,
      "apps/native-shell/tauri/icons/icon.png": 512,
      "apps/native-shell/apple/Assets.xcassets/AppIcon.appiconset/AppIcon-512@2x.png":
        1024,
      "site/assets/cowboy-brand-icon-v6.png": 256,
      "site/assets/cowboy-readme-icon-v6.png": 256,
    })
  ) assertEquals(await pngSize(path), [size, size], path);
  const expected = await bytes(
    "web/public/app-icons/v5/palette-103/icon-512.png",
  );
  for (
    const path of [
      "web/public/cowboy-app-icon-512-v6.png",
    ]
  ) {
    assertEquals(await bytes(path), expected, path);
  }
  // Tauri embeds RGBA bytes; Apple launcher assets use opaque RGB. Equal
  // artwork does not mean these platform encodings have equal file bytes.
  for (
    const name of ["32x32.png", "128x128.png", "128x128@2x.png", "icon.png"]
  ) {
    assertEquals((await bytes(`apps/native-shell/tauri/icons/${name}`))[25], 6);
  }
  assertEquals(
    await bytes("web/public/apple-touch-icon.png"),
    await bytes("web/public/cowboy-app-icon-180-v6.png"),
  );
  assertEquals(
    await bytes("web/public/maskable-512.png"),
    await bytes("web/public/cowboy-app-icon-maskable-512-v6.png"),
  );
  assertEquals([
    ...((await bytes("web/public/cowboy-favicon-v6.ico")).subarray(0, 4)),
  ], [0, 0, 1, 0]);
  assertEquals(
    await bytes("web/public/favicon.ico"),
    await bytes("web/public/cowboy-favicon-v6.ico"),
  );
  assertEquals(
    new TextDecoder().decode(
      (await bytes("apps/native-shell/tauri/icons/icon.icns")).subarray(0, 4),
    ),
    "icns",
  );
  assertEquals(
    await bytes("apps/native-shell/tauri/icons/icon.icns"),
    await bytes("apps/macos-installer/Resources/Cowboy.icns"),
  );
});

Deno.test("entry points use the current icon and the service worker changes generation", async () => {
  for (const path of ["README.md", "README.zh-CN.md"]) {
    assert(
      (await read(path)).includes("site/assets/cowboy-readme-icon-v6.png"),
    );
  }
  for (const path of ["web/index.html", "web/admin.html"]) {
    const text = await read(path);
    assert(text.includes("/cowboy-favicon-v6.ico"));
    assert(text.includes("/cowboy-app-icon-512-v6.png"));
  }
  const index = await read("web/index.html");
  assert(index.includes("/cowboy-app-icon-180-v6.png"));
  assert(index.includes("/manifest.webmanifest?v=cowboy-v1663"));
  const manifest = JSON.parse(await read("web/public/manifest.webmanifest"));
  assertEquals(manifest.id, "/");
  assertEquals(manifest.start_url, "/");
  assert(
    manifest.icons.every((p: { src: string }) =>
      p.src.includes("/palette-103/")
    ),
  );
  const sw = await read("web/public/sw.js");
  assert(sw.includes('const VERSION = "cowboy-v1663"'));
  assert(sw.includes('icon: "/cowboy-app-icon-192-v6.png"'));
});

Deno.test("native alternate icon declarations cover the complete web catalog", async () => {
  const rows = JSON.parse(await read("web/src/appIconCatalog.json")) as {
    id: string;
  }[];
  for (const icon of rows) {
    const base = "apps/native-shell/apple/Assets.xcassets/Cowboy-" + icon.id +
      ".appiconset/";
    const content = JSON.parse(await read(base + "Contents.json"));
    assertEquals(content.images[0].platform, "ios");
    assertEquals(await pngSize(base + "icon.png"), [1024, 1024]);
  }
  const project = await read("apps/native-shell/apple/project.yml");
  assert(
    project.includes("ASSETCATALOG_COMPILER_INCLUDE_ALL_APPICON_ASSETS: YES"),
  );
  assert(project.includes("ASSETCATALOG_COMPILER_APPICON_NAME: AppIcon"));
  const bridge = await read(
    "apps/native-shell/apple/Sources/cowboy-app/CowboyAppIconBridge.mm",
  );
  assert(bridge.includes("message.frameInfo.isMainFrame"));
  assert(bridge.includes("cowboy.stormbird.xyz"));
  assert(bridge.includes("cowboyBundledAlternateIcons()[name] == nil"));
  assert(bridge.includes("setAlternateIconName:name"));
});
