import {
  assert,
  assertEquals,
  assertRejects,
  assertThrows,
} from "jsr:@std/assert";
import {
  APP_ICON_GROUPS,
  APP_ICONS,
  appIcon,
  appIconAppearanceAsset,
  appIconAsset,
  appIconInstallPath,
  appIconTabAsset,
  currentAppIcon,
  DEFAULT_APP_ICON,
  filterAppIcons,
  parseNativeAppIconState,
  selectAppIcon,
} from "./appIcons.ts";

Deno.test("icon catalog preserves Lilac Flow default and all fifty color styles", () => {
  assertEquals(APP_ICONS.length, 50);
  assertEquals(APP_ICON_GROUPS.length, 2);
  assert(APP_ICON_GROUPS.every((group) => group.styles.length === 25));
  assertEquals(
    new Set(APP_ICONS.map((icon) => icon.id)).size,
    APP_ICONS.length,
  );
  assertEquals(appIcon(DEFAULT_APP_ICON).number, 26);
  assertEquals(appIcon(DEFAULT_APP_ICON).crown, "#D8C0FF");
  assertEquals(appIcon(DEFAULT_APP_ICON).brim, "#9776DB");
  assertEquals(appIcon(DEFAULT_APP_ICON).background, "#211B34");
  assertEquals(appIcon("original-001").id, "original-001");
  assert(!APP_ICONS.some((p) => p.id === "original-001"));
  for (const icon of APP_ICONS) {
    assert(/^(original|palette|curlseal)-\d{3}$/.test(icon.id));
  }
});

Deno.test("icon paths fail closed for untrusted stored values", () => {
  for (
    const value of [
      "../../secret",
      "https://evil.invalid/image.png",
      "__proto__",
      "",
      "palette-999",
    ]
  ) {
    assertEquals(appIcon(value).id, DEFAULT_APP_ICON);
    assertEquals(
      appIconAsset(value),
      `/app-icons/v10/${DEFAULT_APP_ICON}/icon-192.png`,
    );
    assertEquals(
      appIconInstallPath(value),
      `/app-icons/v10/${DEFAULT_APP_ICON}/install.html`,
    );
  }
});

Deno.test("icon filtering considers both pieces, background tone, and hex search", () => {
  assert(
    filterAppIcons({ family: "purple", tone: "dark" }).some((p) =>
      p.id === DEFAULT_APP_ICON
    ),
  );
  assert(
    filterAppIcons({ family: "purple", tone: "dark" }).some((p) =>
      p.id === DEFAULT_APP_ICON
    ),
  );
  assert(
    !filterAppIcons({ tone: "light" }).some((p) => p.id === DEFAULT_APP_ICON),
  );
  assertEquals(filterAppIcons({ query: "curlseal-026" }).map((p) => p.id), [
    DEFAULT_APP_ICON,
  ]);
  assert(
    filterAppIcons({ query: "#d8c0ff" }).some((p) => p.id === DEFAULT_APP_ICON),
  );
  assertEquals(filterAppIcons({ query: "not-a-palette" }), []);
  assertEquals(filterAppIcons({ query: "26" }).map((p) => p.id), [
    DEFAULT_APP_ICON,
  ]);
});

Deno.test("native icon replies reject errors, malformed data, and unknown current state", () => {
  for (
    const raw of [null, {}, { ok: false, error: "Denied" }, { ok: true }, {
      ok: true,
      supported: true,
      current: "future-icon",
      available: [],
    }]
  ) {
    assertThrows(() => parseNativeAppIconState(raw));
  }
  assertEquals(
    parseNativeAppIconState({
      ok: true,
      supported: true,
      current: DEFAULT_APP_ICON,
      available: [DEFAULT_APP_ICON, "future-icon"],
    }).available,
    [DEFAULT_APP_ICON],
  );
});

Deno.test("native icon selection commits only after the OS reports the selected icon", async () => {
  const root = globalThis as typeof globalThis & {
    __cowboyNativeShell?: boolean;
    __cowboyAppIcon?: (request: unknown) => Promise<unknown>;
  };
  const previous = currentAppIcon();
  root.__cowboyNativeShell = true;
  try {
    await assertRejects(() => selectAppIcon("not-an-icon"));
    await assertRejects(() => selectAppIcon("curlseal-004"));
    assertEquals(currentAppIcon(), previous);
    root.__cowboyAppIcon = () =>
      Promise.resolve({ ok: false, error: "Cancelled" });
    await assertRejects(() => selectAppIcon("curlseal-004"));
    assertEquals(currentAppIcon(), previous);
    root.__cowboyAppIcon = () =>
      Promise.resolve({
        ok: true,
        supported: true,
        current: previous,
        available: [previous],
      });
    await assertRejects(() => selectAppIcon("curlseal-004"));
    assertEquals(currentAppIcon(), previous);
    root.__cowboyAppIcon = (request) =>
      Promise.resolve({
        ok: true,
        supported: true,
        current: (request as { id: string }).id,
        available: ["curlseal-004", DEFAULT_APP_ICON],
      });
    await selectAppIcon("curlseal-004");
    assertEquals(currentAppIcon(), "curlseal-004");
    await selectAppIcon(previous);
  } finally {
    delete root.__cowboyNativeShell;
    delete root.__cowboyAppIcon;
  }
});

Deno.test("every icon has installable files with a shared identity and an isolated handoff", async () => {
  for (const icon of APP_ICONS) {
    const base = new URL(
      `../public/app-icons/v10/${icon.id}/`,
      import.meta.url,
    );
    const manifest = JSON.parse(
      await Deno.readTextFile(new URL("manifest.webmanifest", base)),
    );
    assertEquals(manifest.id, "/");
    assertEquals(manifest.scope, "/");
    assertEquals(manifest.start_url, `/?app-icon=${icon.id}`);
    assert(
      manifest.icons.some((p: { purpose: string }) => p.purpose === "maskable"),
    );
    for (const size of [96, 180, 192, 512]) {
      const bytes = await Deno.readFile(new URL(`icon-${size}.png`, base));
      const view = new DataView(bytes.buffer);
      assertEquals([view.getUint32(16), view.getUint32(20)], [size, size]);
    }
    const install = await Deno.readTextFile(new URL("install.html", base));
    assert(install.includes('rel="apple-touch-icon"'));
    assert(install.includes(`/?app-icon=${icon.id}`));
    assert(!install.includes("tmpfiles.org"));
  }
});

Deno.test("Curlseal previews preserve the approved artwork in both modes", () => {
  // The opaque vector, not the 192px raster the installable files still use.
  assertEquals(
    appIconAppearanceAsset(DEFAULT_APP_ICON, false),
    "/app-icons/v10/curlseal-026/icon.svg",
  );
  assertEquals(
    appIconAppearanceAsset(DEFAULT_APP_ICON, true),
    appIconAppearanceAsset(DEFAULT_APP_ICON, false),
  );
  assertEquals(
    appIconAsset(DEFAULT_APP_ICON, 192),
    "/app-icons/v10/curlseal-026/icon-192.png",
  );
  for (const icon of APP_ICONS.filter((icon) => icon.id !== DEFAULT_APP_ICON)) {
    assertEquals(
      appIconAppearanceAsset(icon.id, false),
      appIconAppearanceAsset(icon.id, true),
    );
  }
});

Deno.test("tab marks are independent of opaque installation icons", () => {
  assertEquals(
    appIconTabAsset(DEFAULT_APP_ICON),
    "/app-icons/v10/curlseal-026/favicon.svg",
  );
  assertEquals(
    appIconTabAsset("palette-054"),
    "/app-icons/v9/palette-054/favicon.svg",
  );
  assertEquals(
    appIconTabAsset("original-001"),
    appIconAsset("original-001", 192),
  );
});
