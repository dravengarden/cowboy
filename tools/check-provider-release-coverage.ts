interface PluginManifest {
  id: string;
  version: string;
  publisher: string;
  kind: string;
}

interface RuntimeComponent {
  artifact_url: string;
  artifact_digest: string;
}

interface PluginRelease {
  release_schema: number;
  plugin_id: string;
  plugin_version: string;
  package_digest: string;
  artifact_digest: string;
  artifact_url: string;
  publisher: string;
  signature: string;
  runtime_artifacts: Array<{ components: RuntimeComponent[] }>;
}

interface PublicationReceipt {
  schema_version: number;
  plugin_id: string;
  plugin_version: string;
  package_digest: string;
  artifact_digest: string;
  publisher: string;
  catalog_package: string;
  catalog_release: string;
}

export interface ProviderReleaseCoverage {
  plugin_id: string;
  plugin_version: string;
  covered: boolean;
  detail: string;
}

const exactDigest = /^sha256:([a-f0-9]{64})$/;

export async function checkProviderReleaseCoverage(
  pluginRoot: string,
  catalogRoot: string,
): Promise<ProviderReleaseCoverage[]> {
  const requirements = await providerRequirements(pluginRoot);
  const releases = await catalogReleases(catalogRoot);
  const coverage: ProviderReleaseCoverage[] = [];
  for (const requirement of requirements) {
    const candidates = releases.filter(({ release }) =>
      release.plugin_id === requirement.id &&
      release.plugin_version === requirement.version &&
      release.publisher === requirement.publisher
    );
    let failure = "no exact signed release is published";
    let covered = false;
    for (const candidate of candidates) {
      try {
        await validatePublishedRelease(
          catalogRoot,
          candidate.path,
          candidate.release,
        );
        covered = true;
        failure = `signed release ${candidate.release.artifact_digest}`;
        break;
      } catch (cause) {
        failure = cause instanceof Error ? cause.message : String(cause);
      }
    }
    coverage.push({
      plugin_id: requirement.id,
      plugin_version: requirement.version,
      covered,
      detail: failure,
    });
  }
  return coverage;
}

async function providerRequirements(
  pluginRoot: string,
): Promise<PluginManifest[]> {
  const requirements: PluginManifest[] = [];
  for await (const entry of Deno.readDir(pluginRoot)) {
    if (!entry.isDirectory) continue;
    const path = join(pluginRoot, entry.name, "plugin.json");
    if (!await exists(path)) continue;
    const manifest = await readJson<PluginManifest>(path);
    if (manifest.kind === "agent_provider") requirements.push(manifest);
  }
  return requirements.sort((left, right) => left.id.localeCompare(right.id));
}

async function catalogReleases(
  catalogRoot: string,
): Promise<Array<{ path: string; release: PluginRelease }>> {
  const releases: Array<{ path: string; release: PluginRelease }> = [];
  for await (const entry of Deno.readDir(catalogRoot)) {
    if (!entry.isFile || !entry.name.endsWith(".release.json")) continue;
    const path = join(catalogRoot, entry.name);
    releases.push({ path, release: await readJson<PluginRelease>(path) });
  }
  return releases;
}

async function validatePublishedRelease(
  catalogRoot: string,
  releasePath: string,
  release: PluginRelease,
): Promise<void> {
  assert(release.release_schema === 1, "unsupported release schema");
  assert(release.signature.trim().length > 0, "release is unsigned");
  const artifactDigest = digestValue(release.artifact_digest);
  digestValue(release.package_digest);
  const stem = releasePath.slice(0, -".release.json".length);
  const packagePath = `${stem}.cowboy-plugin`;
  assert(await exists(packagePath), "catalog package is missing");
  await validateFileDigest(packagePath, release.package_digest);
  await validateArtifact(
    catalogRoot,
    release.artifact_url,
    release.package_digest,
  );
  for (const target of release.runtime_artifacts) {
    for (const component of target.components) {
      await validateArtifact(
        catalogRoot,
        component.artifact_url,
        component.artifact_digest,
      );
    }
  }
  const publisherKey = join(
    catalogRoot,
    "trusted-publishers",
    `${release.publisher}.pub`,
  );
  assert(await exists(publisherKey), "trusted publisher key is missing");
  assert(
    (await Deno.readTextFile(publisherKey)).trim().startsWith("ssh-ed25519 "),
    "trusted publisher key is not Ed25519",
  );
  const receiptPath = join(
    catalogRoot,
    "receipts",
    `${release.plugin_id}-${release.plugin_version}-${artifactDigest}.json`,
  );
  const receipt = await readJson<PublicationReceipt>(receiptPath);
  assert(receipt.schema_version === 1, "unsupported publication receipt");
  for (
    const key of [
      "plugin_id",
      "plugin_version",
      "package_digest",
      "artifact_digest",
      "publisher",
    ] as const
  ) {
    assert(
      receipt[key] === release[key],
      `publication receipt ${key} mismatch`,
    );
  }
  assert(
    receipt.catalog_package === packagePath,
    "receipt package path mismatch",
  );
  assert(
    receipt.catalog_release === releasePath,
    "receipt release path mismatch",
  );
}

async function validateArtifact(
  catalogRoot: string,
  artifactUrl: string,
  artifactDigest: string,
): Promise<void> {
  const digest = digestValue(artifactDigest);
  const url = new URL(artifactUrl);
  const parts = url.pathname.split("/").filter(Boolean);
  assert(url.protocol === "https:", "artifact URL is not HTTPS");
  assert(
    parts.at(-3) === "plugin-artifacts",
    "artifact URL is outside publication route",
  );
  assert(parts.at(-2) === digest, "artifact URL digest mismatch");
  const name = parts.at(-1) ?? "";
  assert(/^[A-Za-z0-9._-]+$/.test(name), "artifact URL filename is invalid");
  const path = join(catalogRoot, "artifacts", digest, name);
  assert(await exists(path), `published artifact is missing: ${name}`);
  await validateFileDigest(path, artifactDigest);
}

async function validateFileDigest(
  path: string,
  expected: string,
): Promise<void> {
  const output = await new Deno.Command("sha256sum", {
    args: ["--", path],
    clearEnv: true,
    stdout: "piped",
    stderr: "piped",
  }).output();
  assert(
    output.success,
    `could not hash published artifact: ${
      new TextDecoder().decode(output.stderr).trim()
    }`,
  );
  const actual =
    new TextDecoder().decode(output.stdout).trim().split(/\s+/, 1)[0];
  assert(
    `sha256:${actual}` === expected,
    `published artifact digest mismatch: ${path}`,
  );
}

function digestValue(value: string): string {
  const match = exactDigest.exec(value);
  if (!match) throw new Error(`invalid SHA-256 digest ${value}`);
  return match[1]!;
}

async function readJson<T>(path: string): Promise<T> {
  return JSON.parse(await Deno.readTextFile(path)) as T;
}

async function exists(path: string): Promise<boolean> {
  try {
    await Deno.lstat(path);
    return true;
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return false;
    throw error;
  }
}

function join(...parts: string[]): string {
  return parts.map((part, index) =>
    index === 0 ? part.replace(/\/+$/, "") : part.replace(/^\/+|\/+$/g, "")
  ).filter(Boolean).join("/");
}

function assert(condition: boolean, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

if (import.meta.main) {
  const catalogRoot = Deno.args[0] ?? "";
  const pluginRoot = Deno.args[1] ?? "plugins";
  if (!catalogRoot.startsWith("/")) {
    throw new Error("Catalog root must be absolute");
  }
  const coverage = await checkProviderReleaseCoverage(pluginRoot, catalogRoot);
  console.log(
    JSON.stringify({ catalog_root: catalogRoot, providers: coverage }, null, 2),
  );
  const missing = coverage.filter((entry) => !entry.covered);
  if (missing.length > 0) {
    throw new Error(
      `missing current signed Provider releases: ${
        missing.map((entry) => `${entry.plugin_id}@${entry.plugin_version}`)
          .join(", ")
      }`,
    );
  }
}
