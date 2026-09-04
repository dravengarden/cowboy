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
      html.includes('src="assets/cowboy-brand-mark.png"'),
      "wordmarks should use the background-free Cowboy brand icon",
    );
    const controlGroupStart = html.indexOf('<div class="nav-actions"');
    const navToggleStart = html.indexOf('class="nav-toggle"');
    const themeToggleStart = html.indexOf('class="theme-toggle"');
    const controlGroupEnd = html.indexOf("</div>", controlGroupStart);
    assert(
      controlGroupStart >= 0 &&
        navToggleStart > controlGroupStart &&
        themeToggleStart > navToggleStart &&
        controlGroupEnd > themeToggleStart,
      "mobile navigation and theme controls should share one compact control group",
    );
    assert(
      html.includes('class="site-navigation-github"'),
      "mobile navigation should retain the GitHub destination outside the header controls",
    );
    assertEquals(
      (html.match(/class="theme-icon"/gu) ?? []).length,
      1,
      "theme control should use one restrained appearance icon",
    );
    assert(
      html.includes('class="theme-icon-fill"') &&
        !html.includes("theme-icon-moon") &&
        !html.includes("theme-icon-sun"),
      "theme control should use the half-light appearance mark",
    );
    assert(
      html.includes('aria-label="Machine 01"') &&
        html.includes('aria-label="Machine 02"'),
      "hero should use generic demo Machine labels",
    );
    assert(
      html.includes("<i></i> Your Machine</span><strong>Connect next</strong>"),
      "topology should invite a generic next Machine",
    );
    assert(
      html.includes('data-agent-provider="claude-code"') &&
        html.includes('data-agent-provider="deepseek"'),
      "hero control plane should include Claude and DeepSeek",
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
