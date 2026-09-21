import { assert, assertEquals } from "jsr:@std/assert";
import { appIconAppearanceAsset } from "./appIcons.ts";

const repoRoot = new URL("../../", import.meta.url);
const read = (path: string) => Deno.readTextFile(new URL(path, repoRoot));
const bytes = (path: string) => Deno.readFile(new URL(path, repoRoot));

async function pngSize(path: string): Promise<[number, number]> {
  const data = await bytes(path);
  assertEquals([...data.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  return [view.getUint32(16), view.getUint32(20)];
}

Deno.test("Lilac Flow default exports cover web, native, Manager and website", async () => {
  for (
    const [path, size] of Object.entries({
      "assets/brand/cowboy-logo.png": 1024,
      "web/public/cowboy-app-icon-180-v10.png": 180,
      "web/public/cowboy-app-icon-192-v10.png": 192,
      "web/public/cowboy-app-icon-512-v10.png": 512,
      "web/public/cowboy-app-icon-maskable-512-v10.png": 512,
      "apps/native-shell/tauri/icons/icon.png": 512,
      "apps/native-shell/apple/Assets.xcassets/AppIcon.appiconset/AppIcon-512@2x.png":
        1024,
      "site/assets/cowboy-brand-icon-v10.png": 512,
    })
  ) assertEquals(await pngSize(path), [size, size], path);
  const expected = await bytes(
    "web/public/app-icons/v10/curlseal-026/icon-512.png",
  );
  for (
    const path of [
      "web/public/cowboy-app-icon-512-v10.png",
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
    await bytes("web/public/cowboy-app-icon-180-v10.png"),
  );
  assertEquals(
    await bytes("web/public/maskable-512.png"),
    await bytes("web/public/cowboy-app-icon-maskable-512-v10.png"),
  );
  assertEquals([
    ...((await bytes("web/public/cowboy-favicon-v10.ico")).subarray(0, 4)),
  ], [0, 0, 1, 0]);
  assertEquals(
    await bytes("web/public/favicon.ico"),
    await bytes("web/public/cowboy-favicon-v10.ico"),
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
      (await read(path)).includes("site/assets/cowboy-readme-mark-v10.svg"),
    );
  }
  for (const path of ["web/index.html", "web/admin.html"]) {
    const text = await read(path);
    assert(text.includes("/cowboy-favicon-v10.ico"));
    assert(text.includes("/cowboy-favicon-v10.svg"));
  }
  const index = await read("web/index.html");
  assert(index.includes("/cowboy-app-icon-180-v10.png"));
  assert(index.includes("/manifest.webmanifest?v=cowboy-v1736"));
  const manifest = JSON.parse(await read("web/public/manifest.webmanifest"));
  assertEquals(manifest.id, "/");
  assertEquals(manifest.start_url, "/");
  assert(
    manifest.icons.every((p: { src: string }) =>
      p.src.includes("/curlseal-026/")
    ),
  );
  const sw = await read("web/public/sw.js");
  const version = /const VERSION = "cowboy-v([1-9]\d*)"/.exec(sw);
  assert(version && Number(version[1]) >= 1683);
  assert(sw.includes('icon: "/cowboy-app-icon-192-v10.png"'));
});

// SideStore re-signs every bundled file on device under a free team, so the
// asset catalog carries only icons the picker can actually select. Archived
// catalog entries keep their web artwork and lose Home Screen switching.
Deno.test("native alternate icons ship exactly the curated picker styles", async () => {
  const styles = JSON.parse(await read("web/src/appIconStyles.json")) as {
    groups: { styles: { id: string }[] }[];
  };
  const curated = styles.groups.flatMap((group) =>
    group.styles.map((style) => style.id)
  );
  for (const id of curated) {
    const base =
      `apps/native-shell/apple/Assets.xcassets/Cowboy-${id}.appiconset/`;
    const content = JSON.parse(await read(base + "Contents.json"));
    assertEquals(content.images[0].platform, "ios");
    assertEquals(await pngSize(base + "icon.png"), [1024, 1024]);
    // The Home Screen icon must stay raster, but the picker renders the same
    // artwork as its opaque vector. Keep both exports present for every style.
    const svg = await read(
      appIconAppearanceAsset(id, false).replace(
        "/app-icons/",
        "web/public/app-icons/",
      ),
    );
    assert(svg.startsWith("<svg "), id);
    assert(svg.includes('viewBox="0 0 1024 1024"'), id);
    // Unlike the tab mark, a preview shows the Home Screen icon as installed:
    // opaque, and never re-tinted by the page's color scheme.
    assert(svg.includes('<rect width="1024" height="1024"'), id);
    assert(!svg.includes("prefers-color-scheme"), id);
  }
  const bundled: string[] = [];
  for await (
    const entry of Deno.readDir(
      new URL("apps/native-shell/apple/Assets.xcassets", repoRoot),
    )
  ) {
    const id = /^Cowboy-(.+)\.appiconset$/.exec(entry.name)?.[1];
    if (id !== undefined) bundled.push(id);
  }
  assertEquals(bundled.sort(), [...curated].sort());
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

// Neon was the default before curlseal-026, so a stored preference still
// resolves. Its Home Screen variant is archived; the paired web appearances
// that appIconAppearanceAsset() serves are not.
Deno.test("archived Neon retains its light and dark web appearances", async () => {
  for (
    const path of [
      "web/public/app-icons/v6/palette-103/icon-light-192.png",
      "web/public/app-icons/v5/palette-103/icon-192.png",
    ]
  ) assertEquals(await pngSize(path), [192, 192], path);
  assertEquals(
    appIconAppearanceAsset("palette-103", false),
    "/app-icons/v6/palette-103/icon-light-192.png",
  );
  assertEquals(
    appIconAppearanceAsset("palette-103", true),
    "/app-icons/v5/palette-103/icon-192.png",
  );
});

Deno.test("tab SVGs are transparent vectors and ICO has native browser frames", async () => {
  const svg = await read("web/public/cowboy-favicon-v10.svg");
  assert(svg.includes("prefers-color-scheme:dark"));
  assert(
    svg.includes('<path class="crown"') && svg.includes('<path class="brim"'),
  );
  assert(!svg.includes("<image") && !svg.includes("<rect"));
  const ico = await bytes("web/public/cowboy-favicon-v10.ico");
  const view = new DataView(ico.buffer, ico.byteOffset, ico.byteLength);
  assertEquals(view.getUint16(4, true), 3);
  assertEquals([ico[6], ico[22], ico[38]], [16, 32, 48]);
});
