import {
  assert,
  assertEquals,
  assertRejects,
  assertThrows,
} from "jsr:@std/assert";
import {
  APP_ICONS,
  appIcon,
  appIconAsset,
  appIconInstallPath,
  currentAppIcon,
  DEFAULT_APP_ICON,
  filterAppIcons,
  parseNativeAppIconState,
  selectAppIcon,
} from "./appIcons.ts";

Deno.test("icon catalog preserves default 54 and complete, uniquely addressed collections", () => {
  assert(APP_ICONS.length >= 200);
  assertEquals(
    new Set(APP_ICONS.map((icon) => icon.id)).size,
    APP_ICONS.length,
  );
  assertEquals(appIcon(DEFAULT_APP_ICON).number, 54);
  assertEquals(appIcon(DEFAULT_APP_ICON).crown, "#E8BDD0");
  assertEquals(appIcon(DEFAULT_APP_ICON).brim, "#BDD2ED");
  assertEquals(appIcon(DEFAULT_APP_ICON).background, "#232831");
  assertEquals(APP_ICONS.filter((p) => p.collection === "original").length, 50);
  for (const icon of APP_ICONS) {
    assert(/^(original|palette)-\d{3}$/.test(icon.id));
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
      `/app-icons/v5/${DEFAULT_APP_ICON}/icon-192.png`,
    );
    assertEquals(
      appIconInstallPath(value),
      `/app-icons/v5/${DEFAULT_APP_ICON}/install.html`,
    );
  }
});

Deno.test("icon filtering considers both pieces, background tone, and hex search", () => {
  assert(
    filterAppIcons({ family: "pink", tone: "dark" }).some((p) =>
      p.id === DEFAULT_APP_ICON
    ),
  );
  assert(
    filterAppIcons({ family: "blue", tone: "dark" }).some((p) =>
      p.id === DEFAULT_APP_ICON
    ),
  );
  assert(
    !filterAppIcons({ tone: "light" }).some((p) => p.id === DEFAULT_APP_ICON),
  );
  assertEquals(filterAppIcons({ query: "palette-054" }).map((p) => p.id), [
    DEFAULT_APP_ICON,
  ]);
  assert(
    filterAppIcons({ query: "#e8bdd0" }).some((p) => p.id === DEFAULT_APP_ICON),
  );
  assertEquals(filterAppIcons({ query: "not-a-palette" }), []);
  assertEquals(filterAppIcons({ query: "54" }).map((p) => p.id), [
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
    await assertRejects(() => selectAppIcon("palette-051"));
    assertEquals(currentAppIcon(), previous);
    root.__cowboyAppIcon = () =>
      Promise.resolve({ ok: false, error: "Cancelled" });
    await assertRejects(() => selectAppIcon("palette-051"));
    assertEquals(currentAppIcon(), previous);
    root.__cowboyAppIcon = () =>
      Promise.resolve({
        ok: true,
        supported: true,
        current: previous,
        available: [previous],
      });
    await assertRejects(() => selectAppIcon("palette-051"));
    assertEquals(currentAppIcon(), previous);
    root.__cowboyAppIcon = (request) =>
      Promise.resolve({
        ok: true,
        supported: true,
        current: (request as { id: string }).id,
        available: ["palette-051", DEFAULT_APP_ICON],
      });
    await selectAppIcon("palette-051");
    assertEquals(currentAppIcon(), "palette-051");
    await selectAppIcon(previous);
  } finally {
    delete root.__cowboyNativeShell;
    delete root.__cowboyAppIcon;
  }
});

Deno.test("every icon has installable files with a shared identity and an isolated handoff", async () => {
  for (const icon of APP_ICONS) {
    const base = new URL(`../public/app-icons/v5/${icon.id}/`, import.meta.url);
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
