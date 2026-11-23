import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { join, resolve } from "node:path";

export interface ExactDependency {
  id: string;
  version: string;
  source: string;
  integrity: string;
  private: boolean;
}

export interface ProviderSource {
  id: string;
  version: string;
  runtime: { dependencies: ExactDependency[] };
}

export type AuditStatus =
  | "current"
  | "upgrade_available"
  | "integrity_mismatch"
  | "workspace_local"
  | "workspace_drift"
  | "git_pinned"
  | "manual_git_review"
  | "upstream_check_failed";

export interface AuditRow {
  provider_id: string;
  provider_version: string;
  dependency_id: string;
  current_version: string;
  candidate_version?: string;
  current_integrity: string;
  candidate_integrity?: string;
  status: AuditStatus;
  source: string;
  release_url?: string;
  detail?: string;
}

export interface AuditReport {
  audited_at: string;
  providers: string[];
  dependencies: AuditRow[];
}

interface NpmMetadata {
  version?: string;
  dist?: {
    integrity?: string;
    tarball?: string;
  };
}

interface GitHubRelease {
  tag_name?: string;
  html_url?: string;
}

export type FetchFn = (input: string, init?: RequestInit) => Promise<Response>;

export type SourceKind =
  | { kind: "npm"; packageName: string }
  | { kind: "antigravity_cli" }
  | { kind: "workspace_local"; relativePath: string }
  | { kind: "git_pinned"; repo: string; rev: string; subdir?: string }
  | { kind: "manual" };

const ANTIGRAVITY_CLI_RELEASE_URL =
  "https://api.github.com/repos/google-antigravity/antigravity-cli/releases/latest";

export function classifySource(source: string): SourceKind {
  const npm = /^https:\/\/registry\.npmjs\.org\/(.+)\/-\/[^/]+\.tgz$/.exec(source);
  if (npm?.[1]) {
    return { kind: "npm", packageName: decodeURIComponent(npm[1]) };
  }
  if (source.startsWith("https://storage.googleapis.com/antigravity-public/antigravity-cli/")) {
    return { kind: "antigravity_cli" };
  }
  if (source.startsWith("cowboy:")) {
    return { kind: "workspace_local", relativePath: source.slice("cowboy:".length) };
  }
  const git = /^git\+ssh:\/\/[^#]+#([a-f0-9]{7,40})(?::(.+))?$/.exec(source);
  if (git?.[1]) {
    return { kind: "git_pinned", repo: source.split("#")[0]!, rev: git[1], subdir: git[2] };
  }
  return { kind: "manual" };
}

export async function auditDependencies(
  options: { repoRoot?: string; requested?: string; offline?: boolean; fetchFn?: FetchFn } = {},
): Promise<AuditReport> {
  const requested = options.requested ?? "all";
  if (requested !== "all" && !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(requested)) {
    throw new Error("Provider id must use lowercase kebab-case");
  }
  const repoRoot = options.repoRoot
    ? resolve(options.repoRoot)
    : resolve(new URL("../../../../", import.meta.url).pathname);
  const providersRoot = new URL(`file://${join(repoRoot, "plugins")}/`);
  const providerIds = requested === "all" ? discoverProviderIds(providersRoot) : [requested];
  const fetchFn = options.fetchFn ?? fetch;
  const rows: AuditRow[] = [];

  for (const providerId of providerIds) {
    const path = new URL(`${providerId}/provider.json`, providersRoot);
    const source = JSON.parse(await readFile(path, "utf8")) as ProviderSource;
    if (source.id !== providerId) {
      throw new Error(`Provider directory identity mismatch: ${providerId}`);
    }
    for (const dependency of source.runtime.dependencies) {
      const classified = classifySource(dependency.source);
      if (classified.kind === "workspace_local") {
        const pkgPath = join(repoRoot, classified.relativePath, "package.json");
        let localVersion: string | undefined;
        if (existsSync(pkgPath)) {
          try {
            const pkg = JSON.parse(readFileSync(pkgPath, "utf8")) as { version?: string };
            localVersion = pkg.version;
          } catch {
            // Fall through to workspace_local without candidate.
          }
        }
        const drifted = localVersion !== undefined && localVersion !== dependency.version;
        rows.push({
          provider_id: source.id,
          provider_version: source.version,
          dependency_id: dependency.id,
          current_version: dependency.version,
          candidate_version: drifted ? localVersion : undefined,
          current_integrity: dependency.integrity,
          status: drifted ? "workspace_drift" : "workspace_local",
          source: dependency.source,
        });
        continue;
      }
      if (classified.kind === "git_pinned") {
        rows.push({
          provider_id: source.id,
          provider_version: source.version,
          dependency_id: dependency.id,
          current_version: dependency.version,
          current_integrity: dependency.integrity,
          status: "git_pinned",
          source: dependency.source,
          detail: `pinned at ${classified.rev.slice(0, 12)}${classified.subdir ? ` (${classified.subdir})` : ""}`,
        });
        continue;
      }
      if (classified.kind === "manual") {
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
      if (options.offline) {
        rows.push({
          provider_id: source.id,
          provider_version: source.version,
          dependency_id: dependency.id,
          current_version: dependency.version,
          current_integrity: dependency.integrity,
          status: "upstream_check_failed",
          source: dependency.source,
          detail: "skipped in offline mode",
        });
        continue;
      }
      if (classified.kind === "antigravity_cli") {
        try {
          const release = await githubLatestRelease(ANTIGRAVITY_CLI_RELEASE_URL, fetchFn);
          const candidate = release.tag_name?.replace(/^v/, "");
          if (!candidate) {
            throw new Error("GitHub release metadata lacked tag_name");
          }
          rows.push({
            provider_id: source.id,
            provider_version: source.version,
            dependency_id: dependency.id,
            current_version: dependency.version,
            candidate_version: candidate,
            current_integrity: dependency.integrity,
            status: candidate === dependency.version ? "current" : "upgrade_available",
            source: dependency.source,
            release_url: release.html_url,
          });
        } catch (error) {
          rows.push({
            provider_id: source.id,
            provider_version: source.version,
            dependency_id: dependency.id,
            current_version: dependency.version,
            current_integrity: dependency.integrity,
            status: "upstream_check_failed",
            source: dependency.source,
            detail: error instanceof Error ? error.message : String(error),
          });
        }
        continue;
      }
      // npm tarball
      try {
        const [current, candidate] = await Promise.all([
          npmMetadata(classified.packageName, dependency.version, fetchFn),
          npmMetadata(classified.packageName, "latest", fetchFn),
        ]);
        if (
          current.version !== dependency.version ||
          !current.dist?.integrity ||
          !current.dist.tarball ||
          !candidate.version ||
          !candidate.dist?.integrity
        ) {
          throw new Error(`npm registry metadata is incomplete for ${classified.packageName}`);
        }
        const integrityMatches =
          current.dist.integrity === dependency.integrity &&
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
      } catch (error) {
        rows.push({
          provider_id: source.id,
          provider_version: source.version,
          dependency_id: dependency.id,
          current_version: dependency.version,
          current_integrity: dependency.integrity,
          status: "upstream_check_failed",
          source: dependency.source,
          detail: error instanceof Error ? error.message : String(error),
        });
      }
    }
  }

  return {
    audited_at: new Date().toISOString(),
    providers: providerIds,
    dependencies: rows,
  };
}

export function discoverProviderIds(providersRoot: URL): string[] {
  return [...readdirSync(providersRoot, { withFileTypes: true })]
    .filter(
      (entry) =>
        entry.isDirectory() && isFile(new URL(`${entry.name}/provider.json`, providersRoot)),
    )
    .map((entry) => entry.name)
    .sort();
}

function isFile(path: URL): boolean {
  try {
    return statSync(path).isFile();
  } catch (cause) {
    if ((cause as { code?: string }).code === "ENOENT") return false;
    throw cause;
  }
}

async function npmMetadata(
  packageName: string,
  selector: string,
  fetchFn: FetchFn,
): Promise<NpmMetadata> {
  const encodedName = encodeURIComponent(packageName);
  const encodedSelector = encodeURIComponent(selector);
  const response = await fetchFn(`https://registry.npmjs.org/${encodedName}/${encodedSelector}`, {
    headers: { accept: "application/json" },
  });
  if (!response.ok) {
    throw new Error(`npm registry returned ${response.status} for ${packageName}@${selector}`);
  }
  return (await response.json()) as NpmMetadata;
}

async function githubLatestRelease(url: string, fetchFn: FetchFn): Promise<GitHubRelease> {
  const response = await fetchFn(url, {
    headers: {
      accept: "application/vnd.github+json",
      "user-agent": "cowboy-dependency-audit",
    },
  });
  if (!response.ok) {
    throw new Error(`GitHub releases returned ${response.status}`);
  }
  return (await response.json()) as GitHubRelease;
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const offline = args.includes("--offline");
  const positional = args.filter((arg) => !arg.startsWith("--"));
  const requested = positional[0] ?? "all";
  const report = await auditDependencies({ requested, offline });
  console.log(JSON.stringify(report, null, 2));
  if (report.dependencies.some((row) => row.status === "integrity_mismatch")) process.exit(2);
}
