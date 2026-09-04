import { resolve, sep } from "node:path";

interface PluginManifest {
  schema_version: number;
  id: string;
  version: string;
  component_release: string;
  publisher: string;
  kind: "agent_provider" | "code_intelligence";
  entrypoint: string;
}

interface GradientStop {
  offset_percent: number;
  color: string;
}

interface ProviderDisplay {
  name: string;
  vendor: string;
  summary: string;
  accent: string;
  secondary_accent: string;
  mark_view_box: string;
  mark_path: string;
  mark_fill?: string;
  mark_gradient?: {
    x1_percent: number;
    y1_percent: number;
    x2_percent: number;
    y2_percent: number;
    stops: GradientStop[];
  };
}

interface ProviderManifest {
  id: string;
  version: string;
  display: ProviderDisplay;
}

export interface SitePlugin {
  id: string;
  version: string;
  componentRelease: string;
  publisher: string;
  kind: PluginManifest["kind"];
  kindLabel: string;
  name: string;
  vendor: string;
  summary: string;
  accentLight: string;
  accentDark: string;
  secondaryAccent: string;
  markViewBox: string;
  markPath: string;
  markMode: "fill" | "stroke";
  markFill?: string;
  markGradient?: ProviderDisplay["mark_gradient"];
}

const PLUGIN_ORDER = [
  "codex",
  "claude-code",
  "gemini",
  "grok",
  "codex-deepseek",
  "claude-deepseek",
  "zed",
];

const CODE_INTELLIGENCE_PRESENTATION: Record<
  string,
  Omit<
    SitePlugin,
    | "id"
    | "version"
    | "componentRelease"
    | "publisher"
    | "kind"
    | "kindLabel"
  >
> = {
  zed: {
    name: "Zed Code Intelligence",
    vendor: "Zed",
    summary:
      "Process-isolated symbols, hover, definitions, and references for every connected worktree.",
    accentLight: "#6E56CF",
    accentDark: "#9E8CFC",
    secondaryAccent: "#168B78",
    markViewBox: "0 0 24 24",
    markPath: "M8 6 2.75 12 8 18M16 6l5.25 6L16 18M14 3l-4 18",
    markMode: "stroke",
  },
};

const REQUIRED_ASSETS = [
  ["site/assets/cowboy-hat-mark-v2.svg", "assets/cowboy-hat-mark.svg"],
  ["site/assets/cowboy-hat-favicon-v2.ico", "assets/favicon.ico"],
  ["site/assets/cowboy-hat-mark-v2-512.png", "assets/cowboy-logo-512.png"],
  [
    "site/assets/cowboy-brand-mark-transparent-v5.png",
    "assets/cowboy-brand-mark.png",
  ],
  [
    "site/assets/cowboy-hero-devices-light-v4.webp",
    "assets/cowboy-hero-devices-light.webp",
  ],
  [
    "site/assets/cowboy-desktop-surface-light-v2.webp",
    "assets/cowboy-desktop-surface-light.webp",
  ],
  [
    "site/assets/cowboy-mobile-light-v2.webp",
    "assets/cowboy-mobile-light.webp",
  ],
] as const;

const PUBLIC_SITE_FORBIDDEN_TEXT = [
  [
    "local filesystem path",
    /\/(?:home|Users|srv|mnt|Volumes)\/[^\s"'<>]+/u,
  ],
  [
    "private IPv4 address",
    /\b(?:10(?:\.\d{1,3}){3}|192\.168(?:\.\d{1,3}){2}|172\.(?:1[6-9]|2\d|3[01])(?:\.\d{1,3}){2})\b/u,
  ],
  ["email address", /\b[\w.%+-]+@[\w.-]+\.[a-z]{2,}\b/iu],
] as const;

export function assertPublicSiteContent(
  label: string,
  content: string | Uint8Array,
): void {
  const text = typeof content === "string"
    ? content
    : new TextDecoder().decode(content);

  for (const [description, pattern] of PUBLIC_SITE_FORBIDDEN_TEXT) {
    if (pattern.test(text)) {
      throw new Error(`${label} contains ${description}`);
    }
  }
}

function assertGenericDemoMachineLabels(html: string): void {
  const heroLabels = [...html.matchAll(
    /<span class="control-machine-name"><i><\/i>\s*<span>([^<]+)<\/span>/gu,
  )].map((match) => match[1].trim());
  const topology = html.match(
    /<div class="topology-machines">([\s\S]*?)<\/div>/u,
  )?.[1] ?? "";
  const topologyLabels = [...topology.matchAll(
    /<article><span><i><\/i>\s*<span(?:\s[^>]*)?>([^<]+)<\/span><\/span>/gu,
  )].map((match) => match[1].trim());

  if (
    heroLabels.join("|") !== "Machine 01|Machine 02|Machine 03"
  ) {
    throw new Error("hero must use generic demo Machine labels");
  }
  if (topologyLabels.join("|") !== "Machine 01|Machine 02|Machine 03") {
    throw new Error("topology must use generic demo Machine labels");
  }
}

function joinPath(...parts: string[]): string {
  return parts
    .filter(Boolean)
    .map((part, index) =>
      index === 0 ? part.replace(/\/+$/u, "") : part.replace(/^\/+|\/+$/gu, "")
    )
    .join("/");
}

function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function assertHexColor(value: string, label: string): string {
  if (!/^#[\da-f]{6}$/iu.test(value)) {
    throw new Error(`${label} must be a six-digit hex color, got ${value}`);
  }
  return value;
}

async function readJson<T>(path: string): Promise<T> {
  return JSON.parse(await Deno.readTextFile(path)) as T;
}

export async function loadSitePlugins(root: string): Promise<SitePlugin[]> {
  const pluginRoot = joinPath(root, "plugins");
  const manifests: PluginManifest[] = [];

  for await (const entry of Deno.readDir(pluginRoot)) {
    if (!entry.isDirectory) continue;
    const path = joinPath(pluginRoot, entry.name, "plugin.json");
    try {
      manifests.push(await readJson<PluginManifest>(path));
    } catch (error) {
      if (error instanceof Deno.errors.NotFound) continue;
      throw error;
    }
  }

  const plugins = await Promise.all(manifests.map(async (manifest) => {
    if (manifest.schema_version !== 1) {
      throw new Error(
        `${manifest.id}: unsupported Plugin schema ${
          String(manifest.schema_version)
        }`,
      );
    }

    const shared = {
      id: manifest.id,
      version: manifest.version,
      componentRelease: manifest.component_release,
      publisher: manifest.publisher,
      kind: manifest.kind,
      kindLabel: manifest.kind === "agent_provider"
        ? "Agent Provider"
        : "Code Intelligence",
    } as const;

    if (manifest.kind === "code_intelligence") {
      const presentation = CODE_INTELLIGENCE_PRESENTATION[manifest.id];
      if (!presentation) {
        throw new Error(
          `${manifest.id}: missing website presentation metadata`,
        );
      }
      return { ...shared, ...presentation };
    }

    const provider = await readJson<ProviderManifest>(
      joinPath(pluginRoot, manifest.id, manifest.entrypoint),
    );
    if (provider.id !== manifest.id || provider.version !== manifest.version) {
      throw new Error(`${manifest.id}: Plugin and Provider identities diverge`);
    }

    const display = provider.display;
    const accent = assertHexColor(display.accent, `${manifest.id} accent`);
    const accentLight = manifest.id === "grok" ? "#44403C" : accent;
    const accentDark = manifest.id === "grok" ? "#E8E4DC" : accent;

    return {
      ...shared,
      name: display.name,
      vendor: display.vendor,
      summary: display.summary,
      accentLight,
      accentDark,
      secondaryAccent: assertHexColor(
        display.secondary_accent,
        `${manifest.id} secondary accent`,
      ),
      markViewBox: display.mark_view_box,
      markPath: display.mark_path,
      markMode: "fill" as const,
      markFill: display.mark_fill,
      markGradient: display.mark_gradient,
    };
  }));

  const order = new Map(PLUGIN_ORDER.map((id, index) => [id, index]));
  plugins.sort((left, right) =>
    (order.get(left.id) ?? Number.MAX_SAFE_INTEGER) -
      (order.get(right.id) ?? Number.MAX_SAFE_INTEGER) ||
    left.id.localeCompare(right.id)
  );
  return plugins;
}

function pluginMark(plugin: SitePlugin): string {
  const gradient = plugin.markGradient;
  let paint = plugin.markFill ?? "var(--plugin-accent)";
  let definitions = "";

  if (gradient) {
    const id = `plugin-gradient-${plugin.id}`;
    paint = `url(#${id})`;
    definitions = `<defs><linearGradient id="${escapeHtml(id)}" x1="${
      String(gradient.x1_percent)
    }%" y1="${String(gradient.y1_percent)}%" x2="${
      String(gradient.x2_percent)
    }%" y2="${String(gradient.y2_percent)}%">${
      gradient.stops.map((stop) =>
        `<stop offset="${String(stop.offset_percent)}%" stop-color="${
          escapeHtml(assertHexColor(stop.color, `${plugin.id} gradient stop`))
        }"></stop>`
      ).join("")
    }</linearGradient></defs>`;
  }

  const path = plugin.markMode === "stroke"
    ? `<path d="${
      escapeHtml(plugin.markPath)
    }" fill="none" stroke="var(--plugin-accent)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"></path>`
    : `<path d="${escapeHtml(plugin.markPath)}" fill="${
      escapeHtml(paint)
    }"></path>`;

  return `<svg viewBox="${
    escapeHtml(plugin.markViewBox)
  }" role="img" aria-label="${
    escapeHtml(plugin.name)
  } mark">${definitions}${path}</svg>`;
}

export function renderPluginCards(plugins: SitePlugin[]): string {
  return plugins.map((plugin, index) => `
            <article
              class="plugin-card reveal"
              data-plugin="${escapeHtml(plugin.id)}"
              data-kind="${escapeHtml(plugin.kind)}"
              style="--plugin-accent-light: ${
    escapeHtml(plugin.accentLight)
  }; --plugin-accent-dark: ${
    escapeHtml(plugin.accentDark)
  }; --plugin-secondary: ${
    escapeHtml(plugin.secondaryAccent)
  }; --reveal-delay: ${String((index % 3) * 70)}ms"
            >
              <div class="plugin-card-topline">
                <span data-i18n="plugins.kind.${
    plugin.kind === "agent_provider" ? "agent" : "code"
  }">${escapeHtml(plugin.kindLabel)}</span>
                <span class="plugin-version">v${
    escapeHtml(plugin.version)
  }</span>
              </div>
              <div class="plugin-identity">
                <span class="plugin-mark">${pluginMark(plugin)}</span>
                <span>
                  <strong>${escapeHtml(plugin.name)}</strong>
                  <small>${escapeHtml(plugin.vendor)}</small>
                </span>
              </div>
              <p data-i18n="plugins.summary.${escapeHtml(plugin.id)}">${
    escapeHtml(plugin.summary)
  }</p>
              <div class="plugin-card-footer">
                <code>${escapeHtml(plugin.id)}</code>
                <span><span data-i18n="plugins.component">component</span> ${
    escapeHtml(plugin.componentRelease)
  }</span>
              </div>
            </article>`).join("");
}

export function renderPluginCatalog(plugins: SitePlugin[]): string {
  return JSON.stringify(
    plugins.map((plugin) => ({
      id: plugin.id,
      name: plugin.name,
      vendor: plugin.vendor,
      summary: plugin.summary,
      kind: plugin.kind,
      version: plugin.version,
      component_release: plugin.componentRelease,
      publisher: plugin.publisher,
      accent: plugin.accentLight,
      secondary_accent: plugin.secondaryAccent,
    })),
    null,
    2,
  ) + "\n";
}

function validatedOutputPath(root: string, requested: string): string {
  const rootPath = resolve(root);
  const output = resolve(rootPath, requested);

  if (
    !requested.trim() || output === rootPath ||
    !output.startsWith(`${rootPath}${sep}`)
  ) {
    throw new Error(`refusing unsafe website output path: ${output}`);
  }
  return output;
}

export async function buildSite(
  root: string,
  requestedOutput = "site-dist",
): Promise<string> {
  const output = validatedOutputPath(root, requestedOutput);
  const plugins = await loadSitePlugins(root);
  const agentCount =
    plugins.filter((plugin) => plugin.kind === "agent_provider").length;
  const codeCount =
    plugins.filter((plugin) => plugin.kind === "code_intelligence").length;
  const template = await Deno.readTextFile(joinPath(root, "site/index.html"));
  const html = template
    .replaceAll("{{PLUGIN_COUNT}}", String(plugins.length))
    .replaceAll("{{AGENT_PLUGIN_COUNT}}", String(agentCount))
    .replaceAll("{{CODE_PLUGIN_COUNT}}", String(codeCount))
    .replace("{{PLUGIN_CARDS}}", renderPluginCards(plugins));
  const catalog = renderPluginCatalog(plugins);
  const staticFiles = await Promise.all(
    ["styles.css", "site.js", "robots.txt", "sitemap.xml"].map(
      async (file) => ({
        file,
        content: await Deno.readTextFile(joinPath(root, `site/${file}`)),
      }),
    ),
  );
  const assets = await Promise.all(
    REQUIRED_ASSETS.map(async ([source, destination]) => ({
      source,
      destination,
      content: await Deno.readFile(joinPath(root, source)),
    })),
  );

  if (/\{\{[A-Z_]+\}\}/u.test(html)) {
    throw new Error(
      "website template still contains an unresolved placeholder",
    );
  }

  assertPublicSiteContent("rendered website", html);
  assertGenericDemoMachineLabels(html);
  assertPublicSiteContent("Plugin catalog", catalog);
  for (const { file, content } of staticFiles) {
    assertPublicSiteContent(file, content);
  }
  for (const { source, content } of assets) {
    assertPublicSiteContent(source, content);
  }

  await Deno.remove(output, { recursive: true }).catch((error) => {
    if (!(error instanceof Deno.errors.NotFound)) throw error;
  });
  await Deno.mkdir(joinPath(output, "assets"), { recursive: true });
  await Deno.writeTextFile(joinPath(output, "index.html"), html);
  await Deno.writeTextFile(joinPath(output, "404.html"), html);
  await Deno.writeTextFile(joinPath(output, ".nojekyll"), "");
  await Deno.writeTextFile(
    joinPath(output, "plugins.json"),
    catalog,
  );

  for (const { file, content } of staticFiles) {
    await Deno.writeTextFile(joinPath(output, file), content);
  }
  for (const { destination, content } of assets) {
    await Deno.writeFile(joinPath(output, destination), content);
  }

  return output;
}

function parseOutputArgument(args: string[]): string {
  if (args.length === 0) return "site-dist";
  if (args.length === 2 && args[0] === "--out" && args[1]) return args[1];
  throw new Error("usage: deno run site/build.ts [--out <directory>]");
}

if (import.meta.main) {
  const root = Deno.cwd();
  const output = await buildSite(root, parseOutputArgument(Deno.args));
  console.log(`Cowboy website built at ${output}`);
}
