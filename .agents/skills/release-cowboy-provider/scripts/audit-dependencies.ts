interface ExactDependency {
  id: string;
  version: string;
  source: string;
  integrity: string;
  private: boolean;
}

interface ProviderSource {
  id: string;
  version: string;
  runtime: { dependencies: ExactDependency[] };
}

interface AuditRow {
  provider_id: string;
  provider_version: string;
  dependency_id: string;
  current_version: string;
  candidate_version?: string;
  current_integrity: string;
  candidate_integrity?: string;
  status:
    | "current"
    | "upgrade_available"
    | "integrity_mismatch"
    | "manual_git_review";
  source: string;
}

interface NpmMetadata {
  version?: string;
  dist?: {
    integrity?: string;
    tarball?: string;
  };
}

if (import.meta.main) await main();

async function main(): Promise<void> {
  const requested = Deno.args[0] ?? "all";
  if (requested !== "all" && !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(requested)) {
    throw new Error("Provider id must use lowercase kebab-case");
  }
  const root = new URL("../../../../", import.meta.url);
  const providersRoot = new URL("providers/", root);
  const providerIds = requested === "all"
    ? discoverProviderIds(providersRoot)
    : [requested];
  const rows: AuditRow[] = [];

  for (const providerId of providerIds) {
    const path = new URL(`${providerId}/provider.json`, providersRoot);
    const source = JSON.parse(
      await Deno.readTextFile(path),
    ) as ProviderSource;
    if (source.id !== providerId) {
      throw new Error(`Provider directory identity mismatch: ${providerId}`);
    }
    for (const dependency of source.runtime.dependencies) {
      const packageName = npmPackageName(dependency.source);
      if (!packageName) {
        rows.push({
          provider_id: source.id,
          provider_version: source.version,
          dependency_id: dependency.id,
          current_version: dependency.version,
          current_integrity: dependency.integrity,
          status: "manual_git_review",
          source: dependency.source,
        });
        continue;
      }
      const [current, candidate] = await Promise.all([
        npmMetadata(packageName, dependency.version),
        npmMetadata(packageName, "latest"),
      ]);
      if (
        current.version !== dependency.version || !current.dist?.integrity ||
        !current.dist.tarball || !candidate.version ||
        !candidate.dist?.integrity
      ) {
        throw new Error(
          `npm registry metadata is incomplete for ${packageName}`,
        );
      }
      const integrityMatches = current.dist.integrity ===
          dependency.integrity &&
        current.dist.tarball === dependency.source;
      rows.push({
        provider_id: source.id,
        provider_version: source.version,
        dependency_id: dependency.id,
        current_version: dependency.version,
        candidate_version: candidate.version,
        current_integrity: dependency.integrity,
        candidate_integrity: candidate.dist.integrity,
        status: !integrityMatches
          ? "integrity_mismatch"
          : candidate.version === dependency.version
          ? "current"
          : "upgrade_available",
        source: dependency.source,
      });
    }
  }

  console.log(JSON.stringify(
    {
      audited_at: new Date().toISOString(),
      providers: providerIds,
      dependencies: rows,
    },
    null,
    2,
  ));
  if (rows.some((row) => row.status === "integrity_mismatch")) Deno.exit(2);
}

export function discoverProviderIds(providersRoot: URL): string[] {
  return [...Deno.readDirSync(providersRoot)]
    .filter((entry) =>
      entry.isDirectory &&
      isFile(new URL(`${entry.name}/provider.json`, providersRoot))
    )
    .map((entry) => entry.name)
    .sort();
}

function isFile(path: URL): boolean {
  try {
    return Deno.statSync(path).isFile;
  } catch (cause) {
    if (cause instanceof Deno.errors.NotFound) return false;
    throw cause;
  }
}

function npmPackageName(source: string): string | null {
  const match = /^https:\/\/registry\.npmjs\.org\/(.+)\/-\/[^/]+\.tgz$/.exec(
    source,
  );
  return match?.[1] ? decodeURIComponent(match[1]) : null;
}

async function npmMetadata(
  packageName: string,
  selector: string,
): Promise<NpmMetadata> {
  const encodedName = encodeURIComponent(packageName);
  const encodedSelector = encodeURIComponent(selector);
  const response = await fetch(
    `https://registry.npmjs.org/${encodedName}/${encodedSelector}`,
    {
      headers: { accept: "application/json" },
    },
  );
  if (!response.ok) {
    throw new Error(
      `npm registry returned ${response.status} for ${packageName}@${selector}`,
    );
  }
  return await response.json() as NpmMetadata;
}
