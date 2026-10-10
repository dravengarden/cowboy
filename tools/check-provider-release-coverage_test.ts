import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { checkProviderReleaseCoverage } from "./check-provider-release-coverage.ts";

test("Provider release coverage requires the exact signed published version", async () => {
  const root = await mkdtemp(join(tmpdir(), "cowboy-provider-coverage-"));
  try {
    const plugins = `${root}/plugins`;
    const catalog = `${root}/catalog`;
    await mkdir(`${plugins}/example`, { recursive: true });
    await mkdir(`${plugins}/zed`, { recursive: true });
    await mkdir(`${catalog}/trusted-publishers`, { recursive: true });
    await mkdir(`${catalog}/receipts`, { recursive: true });
    const packageDigest = `sha256:${await sha256("package")}`;
    const hostBundleDigest = `sha256:${await sha256("host")}`;
    const artifactDigest = `sha256:${"2".repeat(64)}`;
    const artifactValue = artifactDigest.slice("sha256:".length);
    const packageValue = packageDigest.slice("sha256:".length);
    const stem = `${catalog}/example-1.2.3-${artifactValue}`;
    const packagePath = `${stem}.cowboy-plugin`;
    const releasePath = `${stem}.release.json`;
    const hostBundlePath = `${stem}.hostbundle.json`;
    await writeFile(
      `${plugins}/example/plugin.json`,
      JSON.stringify({
        id: "example",
        version: "1.2.3",
        publisher: "cowboy-first-party",
        kind: "agent_provider",
      }),
    );
    await writeFile(
      `${plugins}/zed/plugin.json`,
      JSON.stringify({
        id: "zed",
        version: "9.9.9",
        publisher: "cowboy-first-party",
        kind: "code_intelligence",
      }),
    );
    await writeFile(
      `${catalog}/trusted-publishers/cowboy-first-party.pub`,
      "ssh-ed25519 fixture\n",
    );
    await mkdir(`${catalog}/artifacts/${packageValue}`, {
      recursive: true,
    });
    await writeFile(
      `${catalog}/artifacts/${packageValue}/example.cowboy-plugin`,
      "package",
    );
    await writeFile(packagePath, "package");
    await writeFile(hostBundlePath, "host");
    const release = {
      release_schema: 2,
      plugin_id: "example",
      plugin_version: "1.2.3",
      package_digest: packageDigest,
      artifact_digest: artifactDigest,
      artifact_url:
        `https://cowboy.example/plugin-artifacts/${packageValue}/example.cowboy-plugin`,
      publisher: "cowboy-first-party",
      host_bundle_digest: hostBundleDigest,
      signature: "signed",
      runtime_artifacts: [],
    };
    await writeFile(releasePath, JSON.stringify(release));
    await writeFile(
      `${catalog}/receipts/example-1.2.3-${artifactValue}.json`,
      JSON.stringify({
        schema_version: 1,
        plugin_id: release.plugin_id,
        plugin_version: release.plugin_version,
        package_digest: release.package_digest,
        artifact_digest: release.artifact_digest,
        publisher: release.publisher,
        catalog_package: packagePath,
        catalog_release: releasePath,
      }),
    );

    assertEquals(await checkProviderReleaseCoverage(plugins, catalog), [{
      plugin_id: "example",
      plugin_version: "1.2.3",
      covered: true,
      detail: `signed release ${artifactDigest}`,
    }]);

    const publishedPackage =
      `${catalog}/artifacts/${packageValue}/example.cowboy-plugin`;
    await writeFile(publishedPackage, "tampered");
    const [tampered] = await checkProviderReleaseCoverage(plugins, catalog);
    assertEquals(tampered?.covered, false);
    assertEquals(
      tampered?.detail,
      `published artifact digest mismatch: ${publishedPackage}`,
    );
    await writeFile(publishedPackage, "package");

    await writeFile(hostBundlePath, "tampered");
    const [tamperedHost] = await checkProviderReleaseCoverage(plugins, catalog);
    assertEquals(tamperedHost?.covered, false);
    assertEquals(
      tamperedHost?.detail,
      `published artifact digest mismatch: ${hostBundlePath}`,
    );
    await writeFile(hostBundlePath, "host");

    const nativeRelease = {
      ...release,
      release_schema: 4,
      plugin_kind: "agent_provider",
    };
    await writeFile(releasePath, JSON.stringify(nativeRelease));
    const [native] = await checkProviderReleaseCoverage(plugins, catalog);
    assertEquals(native?.covered, true);

    await writeFile(hostBundlePath, "tampered native host");
    const [nativeHost] = await checkProviderReleaseCoverage(plugins, catalog);
    assertEquals(nativeHost?.covered, false);
    assertEquals(
      nativeHost?.detail,
      `published artifact digest mismatch: ${hostBundlePath}`,
    );
    await writeFile(hostBundlePath, "host");

    for (
      const [invalid, expected] of [
        [{ ...nativeRelease, release_schema: 5 }, "unsupported release schema"],
        [
          { ...nativeRelease, plugin_kind: "workspace_extension" },
          "release schema 4 requires an Agent Provider",
        ],
      ] as const
    ) {
      await writeFile(releasePath, JSON.stringify(invalid));
      const [rejected] = await checkProviderReleaseCoverage(plugins, catalog);
      assertEquals(rejected?.covered, false);
      assertEquals(rejected?.detail, expected);
    }

    await writeFile(
      releasePath,
      JSON.stringify({ ...nativeRelease, host_bundle_digest: undefined }),
    );
    const [unboundHost] = await checkProviderReleaseCoverage(plugins, catalog);
    assertEquals(unboundHost?.covered, false);
    assertEquals(unboundHost?.detail, "catalog host bundle is unbound");
    await rm(hostBundlePath);
    const [withoutHost] = await checkProviderReleaseCoverage(plugins, catalog);
    assertEquals(withoutHost?.covered, true);

    await rm(releasePath);
    assertEquals(await checkProviderReleaseCoverage(plugins, catalog), [{
      plugin_id: "example",
      plugin_version: "1.2.3",
      covered: false,
      detail: "no exact signed release is published",
    }]);
  } finally {
    await rm(root, { recursive: true });
  }
});

async function sha256(value: string): Promise<string> {
  const bytes = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(value),
  );
  return [...new Uint8Array(bytes)].map((byte) =>
    byte.toString(16).padStart(2, "0")
  ).join("");
}
