import {
  assertPublicSiteContent,
  buildSite,
  loadSitePlugins,
  renderPluginCards,
  type SitePlugin,
} from "./build.ts";

const ROOT = decodeURIComponent(new URL("..", import.meta.url).pathname)
  .replace(/\/$/u, "");

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertEquals<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(
      `${message}: expected ${String(expected)}, got ${String(actual)}`,
    );
  }
}

Deno.test("website catalog follows every first-party Plugin manifest", async () => {
  const plugins = await loadSitePlugins(ROOT);
  const manifests: Array<{
    id: string;
    version: string;
    component_release: string;
    kind: SitePlugin["kind"];
  }> = [];

  for await (const entry of Deno.readDir(`${ROOT}/plugins`)) {
    if (!entry.isDirectory) continue;
    try {
      manifests.push(JSON.parse(
        await Deno.readTextFile(`${ROOT}/plugins/${entry.name}/plugin.json`),
      ));
    } catch (error) {
      if (!(error instanceof Deno.errors.NotFound)) throw error;
    }
  }

  assertEquals(
    plugins.length,
    manifests.length,
    "every first-party Plugin should appear",
  );
  assertEquals(
    plugins.filter((plugin) => plugin.kind === "agent_provider").length,
    manifests.filter((manifest) => manifest.kind === "agent_provider").length,
    "Agent Provider count",
  );
  assertEquals(
    plugins.filter((plugin) => plugin.kind === "code_intelligence").length,
    manifests.filter((manifest) => manifest.kind === "code_intelligence")
      .length,
    "code-intelligence Plugin count",
  );

  const currentFirstParty = [
    "codex",
    "claude-code",
    "gemini",
    "grok",
    "codex-deepseek",
    "claude-deepseek",
    "zed",
  ];
  for (const id of currentFirstParty) {
    assert(
      plugins.some((plugin) => plugin.id === id),
      `${id} should remain in the first-party catalog`,
    );
  }

  assertEquals(
    [...plugins].map((plugin) => plugin.id).sort().join(","),
    manifests.map((manifest) => manifest.id).sort().join(","),
    "website and source Plugin identities",
  );

  for (const plugin of plugins) {
    const manifest = manifests.find((candidate) => candidate.id === plugin.id);
    assert(manifest, `${plugin.id} source manifest should exist`);
    assertEquals(plugin.version, manifest.version, `${plugin.id} version`);
    assertEquals(
      plugin.componentRelease,
      manifest.component_release,
      `${plugin.id} component release`,
    );
  }
});

Deno.test("website Plugin cards escape manifest presentation text", () => {
  const plugin: SitePlugin = {
    id: "sample",
    version: "1.0.0",
    componentRelease: "1.0.0",
    publisher: "test",
    kind: "agent_provider",
    kindLabel: "Agent Provider",
    name: "Sample <script>",
    vendor: "Test & Co.",
    summary: 'Safe "summary"',
    accentLight: "#112233",
    accentDark: "#AABBCC",
    secondaryAccent: "#445566",
    markViewBox: "0 0 24 24",
    markPath: "M0 0h24v24H0z",
    markMode: "fill",
  };

  const html = renderPluginCards([plugin]);
  assert(
    !html.includes("Sample <script>"),
    "raw HTML must not reach Plugin cards",
  );
  assert(
    html.includes("Sample &lt;script&gt;"),
    "Plugin name should be escaped",
  );
  assert(html.includes("Test &amp; Co."), "Plugin vendor should be escaped");
  assert(
    html.includes("Safe &quot;summary&quot;"),
    "Plugin summary should be escaped",
  );
});

Deno.test("website privacy guard rejects deployment-specific content", () => {
  for (
    const sample of [
      "/home/example/project",
      "/mnt/work/project",
      "10.23.45.67",
      "owner@example.com",
    ]
  ) {
    let rejected = false;
    try {
      assertPublicSiteContent("test fixture", sample);
    } catch (error) {
      rejected = error instanceof Error && error.message.includes("contains");
    }
    assert(rejected, `${sample} should be rejected from the public site`);
  }

  assertPublicSiteContent(
    "generic product demo",
    "Machine 01 connects through the public Cowboy control plane",
  );
});

Deno.test("repository landing pages use only privacy-safe product artwork", async () => {
  const readme = await Deno.readTextFile(ROOT + "/README.md");
  const contributing = await Deno.readTextFile(ROOT + "/CONTRIBUTING.md");

  assertPublicSiteContent("README.md", readme);
  assertPublicSiteContent("CONTRIBUTING.md", contributing);
  assert(
    readme.includes("site/assets/cowboy-hero-devices-light-v4.webp") &&
      readme.includes("site/assets/cowboy-desktop-surface-light-v2.webp") &&
      readme.includes("site/assets/cowboy-mobile-light-v2.webp") &&
      !readme.includes("docs/screenshots/"),
    "repository landing page should use the abstract public artwork instead of private product captures",
  );
});

Deno.test("website build produces a complete self-contained Pages artifact", async () => {
  const temporary = await Deno.makeTempDir({
    dir: ROOT,
    prefix: ".cowboy-site-test-",
  });
  const output = `${temporary}/site-dist`;

  try {
    await buildSite(ROOT, output);
    const html = await Deno.readTextFile(`${output}/index.html`);
    const notFound = await Deno.readTextFile(`${output}/404.html`);
    const catalog = JSON.parse(
      await Deno.readTextFile(`${output}/plugins.json`),
    ) as Array<{ id: string }>;
    const styles = await Deno.readTextFile(`${output}/styles.css`);
    const script = await Deno.readTextFile(`${output}/site.js`);

    assertEquals(
      notFound,
      html,
      "404 fallback should keep the single-page site usable",
    );
    assertEquals(catalog.length, 7, "published Plugin catalog count");
    assert(
      !/\{\{[A-Z_]+\}\}/u.test(html),
      "all template placeholders should resolve",
    );
    assertEquals(
      (html.match(/<h1[ >]/gu) ?? []).length,
      1,
      "document h1 count",
    );
    assert(
      html.includes('href="#main"'),
      "document should include a skip link",
    );
    assert(
      html.includes('id="plugin-filter-status"'),
      "Plugin filter should announce results",
    );
    assert(
      html.includes('src="assets/cowboy-hero-devices-light.webp"'),
      "hero should use one privacy-safe Desktop and Phone composition",
    );
    assert(
      html.includes('src="assets/cowboy-brand-mark.png"') &&
        (html.match(/class="brand-icon-stage"/gu) ?? []).length === 2,
      "wordmarks should stage the background-free Cowboy brand icon for theme-aware light",
    );
    assert(
      styles.includes("--brand-filter-rest:") &&
        styles.includes("--brand-filter-peak:") &&
        styles.includes("--brand-spectrum:") &&
        styles.includes("--brand-spectrum-opacity:") &&
        styles.includes("@keyframes brand-color-breathe") &&
        styles.includes("@keyframes brand-aura-orbit") &&
        styles.includes("@keyframes brand-spectrum-flow") &&
        styles.includes("animation: brand-spectrum-flow 6s linear infinite") &&
        styles.includes(".site-header .brand-icon-stage::after") &&
        !styles.includes("@keyframes brand-sheen-pass") &&
        styles.includes("animation: none !important"),
      "header brand lighting should continuously flow through a theme-aware spectrum and honor reduced motion",
    );
    const controlGroupStart = html.indexOf('<div class="nav-actions"');
    const navToggleStart = html.indexOf('class="nav-toggle"');
    const preferencesToggleStart = html.indexOf('class="preferences-toggle"');
    assert(
      controlGroupStart >= 0 &&
        navToggleStart > controlGroupStart &&
        preferencesToggleStart > navToggleStart,
      "mobile navigation and preferences should share one compact control group",
    );
    assert(
      html.includes('class="site-navigation-github"'),
      "mobile navigation should retain the GitHub destination outside the header controls",
    );
    assert(
      (html.match(/data-theme-choice=/gu) ?? []).length === 3 &&
        html.includes('data-theme-choice="system"') &&
        html.includes('data-theme-choice="light"') &&
        html.includes('data-theme-choice="dark"') &&
        script.includes('colorSchemeQuery.addEventListener?.("change"'),
      "appearance preferences should offer and live-update System, Light, and Dark modes",
    );
    assert(
      html.includes('data-language="en"') &&
        html.includes('data-language-choice="en"') &&
        html.includes('data-language-choice="zh"') &&
        script.includes('localStorage.setItem("cowboy-site-language"') &&
        script.includes('"preferences.title": "偏好设置"'),
      "English should remain the default while the complete site supports Chinese",
    );
    const translationKeys = new Set(
      [...html.matchAll(
        /data-i18n(?:-(?:aria-label|alt|content))?="([^"]+)"/gu,
      )].map((match) => match[1]),
    );
    for (const key of translationKeys) {
      assert(
        script.includes(`"${key}":`),
        `Chinese translation should cover ${key}`,
      );
    }
    for (const machine of ["01", "02", "03"]) {
      assert(
        html.includes(`Machine ${machine}</span>`),
        `hero should include generic demo Machine ${machine}`,
      );
    }
    assert(
      !html.includes("Machine 04</span>") &&
        !html.includes("Machine 05</span>") &&
        (html.match(/control-machine-multi/gu) ?? []).length === 3 &&
        html.includes('data-agent-provider="codex"') &&
        html.includes('data-agent-provider="claude-code"') &&
        html.includes('data-agent-provider="gemini"') &&
        html.includes('data-agent-provider="deepseek"') &&
        html.includes('data-agent-provider="grok"') &&
        html.includes('data-agent-provider="custom"'),
      "hero should stay at three generic Machines while demonstrating multiple Agents on every Machine",
    );
    assert(
      html.includes('class="hero-client-wiring"') &&
        html.includes('class="control-hub-node"') &&
        html.includes('class="control-fleet-wiring"') &&
        html.includes("seq · route · replay") &&
        html.includes("<b>3M</b>") &&
        html.includes("<b>6A</b>") &&
        !html.includes("control-machine-add") &&
        !html.includes("control-fleet-wiring-mobile"),
      "hero should connect Desktop and Mobile through one Hub to a compact three-Machine topology",
    );
    assert(
      html.includes(
        '<article><span><i></i> <span>Machine 03</span></span><strong data-i18n="topology.connected">Connected</strong>',
      ) &&
        !html.includes('data-i18n="topology.connectNext"'),
      "architecture topology should keep all three generic Machines connected without a fake action",
    );
    assert(
      html.includes('id="safety"') && html.includes("Fail-closed"),
      "website should present safety as a core product property",
    );
    assert(
      !html.includes("floating-provider"),
      "hero should use one adaptive control dock instead of floating Provider cards",
    );
    assert(
      html.includes('src="assets/cowboy-mobile-light.webp"'),
      "mobile surface should use the privacy-safe Cowboy illustration",
    );
    assert(
      html.includes('class="keyboard-promise"') &&
        html.includes("Keyboard controls everything") &&
        !html.includes("Example Cowboy desktop commands"),
      "Desktop should present one Vim-like keyboard promise without shortcut trivia",
    );
    assert(
      html.includes('class="topology-wiring"') &&
        html.includes('class="topology-hub-core"') &&
        html.includes("HTTPS · WebSocket") &&
        html.includes("UDS · outbound WSS") &&
        !html.includes('class="topology-brand-icon"') &&
        !html.includes('class="topology-halo"'),
      "architecture topology should explain real transport and Hub semantics without repeating the logo",
    );
    assert(
      html.includes("Self-hosted remote Agent IDE") &&
        html.includes(
          'One workspace.</span><br><span data-i18n="hero.hosted">Self-hosted.',
        ) &&
        html.includes("Self-host Cowboy on your infrastructure") &&
        script.includes('"hero.hosted":') &&
        script.includes('"hero.ownership":'),
      "hero should make Cowboy's self-hosted single-instance ownership explicit",
    );
    assert(
      html.includes('class="back-to-top" href="#top"') &&
        script.includes('backToTop?.addEventListener("click"') &&
        script.includes("globalThis.scrollTo({"),
      "footer should provide a reliable return-to-top control with an anchor fallback",
    );
    assert(
      html.includes('href="assets/cowboy-hat-mark.svg"'),
      "document should expose the simplified Cowboy hat favicon",
    );
    assert(
      !html.includes("cowboy-desktop.webp") &&
        !html.includes("cowboy-ios-agent.webp"),
      "published HTML should not expose real product captures",
    );
    assert(
      styles.includes('html[data-theme="dark"] .theme-art'),
      "product illustrations should respond to the selected theme",
    );
    assert(
      html.includes("document.documentElement.style.backgroundColor") &&
        html.includes('let initialThemeMode = "system"') &&
        html.includes('matchMedia("(prefers-color-scheme: dark)")') &&
        html.indexOf("document.documentElement.style.backgroundColor") <
          html.indexOf('<link rel="stylesheet"') &&
        script.includes("root.style.backgroundColor = canvas") &&
        script.includes(
          'document.body?.style.setProperty("background-color", canvas)',
        ) &&
        styles.includes(
          ".site-header {\n    background: var(--canvas);\n    backdrop-filter: none;",
        ),
      "selected theme should color the browser edge before and after first paint",
    );

    for (
      const asset of [
        "cowboy-hero-devices-light.webp",
        "cowboy-brand-mark.png",
        "cowboy-desktop-surface-light.webp",
        "cowboy-mobile-light.webp",
      ]
    ) {
      const stat = await Deno.stat(`${output}/assets/${asset}`);
      assert(stat.size < 100_000, `${asset} should remain lightweight`);
    }

    for (const plugin of catalog) {
      assert(
        html.includes(`data-plugin="${plugin.id}"`),
        `${plugin.id} card should be rendered`,
      );
    }

    const references = html.matchAll(/(?:src|href)="([^"#]+)"/gu);
    for (const match of references) {
      const reference = match[1];
      if (/^(?:https?:|mailto:)/u.test(reference)) continue;
      const path = reference.replace(/^\.\//u, "");
      const stat = await Deno.stat(`${output}/${path}`);
      assert(stat.isFile, `${reference} should resolve to a built file`);
    }
  } finally {
    await Deno.remove(temporary, { recursive: true });
  }
});

Deno.test("website build cannot remove a directory outside the repository", async () => {
  let rejected = false;

  try {
    await buildSite(ROOT, "../outside-cowboy-site");
  } catch (error) {
    rejected = error instanceof Error &&
      error.message.includes("refusing unsafe website output path");
  }

  assert(rejected, "an output path outside the repository should be rejected");
});
