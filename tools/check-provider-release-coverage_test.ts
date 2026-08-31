import { assertEquals } from "jsr:@std/assert@1.0.19";
import { checkProviderReleaseCoverage } from "./check-provider-release-coverage.ts";

Deno.test("Provider release coverage requires the exact signed published version", async () => {
  const root = await Deno.makeTempDir({ prefix: "cowboy-provider-coverage-" });
  try {
    const plugins = `${root}/plugins`;
    const catalog = `${root}/catalog`;
    await Deno.mkdir(`${plugins}/example`, { recursive: true });
    await Deno.mkdir(`${plugins}/zed`, { recursive: true });
    await Deno.mkdir(`${catalog}/trusted-publishers`, { recursive: true });
    await Deno.mkdir(`${catalog}/receipts`, { recursive: true });
    const packageDigest = `sha256:${await sha256("package")}`;
    const artifactDigest = `sha256:${"2".repeat(64)}`;
    const artifactValue = artifactDigest.slice("sha256:".length);
    const packageValue = packageDigest.slice("sha256:".length);
    const stem = `${catalog}/example-1.2.3-${artifactValue}`;
    const packagePath = `${stem}.cowboy-plugin`;
    const releasePath = `${stem}.release.json`;
    await Deno.writeTextFile(
      `${plugins}/example/plugin.json`,
      JSON.stringify({
        id: "example",
        version: "1.2.3",
        publisher: "cowboy-first-party",
        kind: "agent_provider",
      }),
    );
    await Deno.writeTextFile(
      `${plugins}/zed/plugin.json`,
      JSON.stringify({
        id: "zed",
        version: "9.9.9",
        publisher: "cowboy-first-party",
        kind: "code_intelligence",
      }),
    );
    await Deno.writeTextFile(
      `${catalog}/trusted-publishers/cowboy-first-party.pub`,
      "ssh-ed25519 fixture\n",
    );
    await Deno.mkdir(`${catalog}/artifacts/${packageValue}`, {
      recursive: true,
    });
    await Deno.writeTextFile(
      `${catalog}/artifacts/${packageValue}/example.cowboy-plugin`,
      "package",
    );
    await Deno.writeTextFile(packagePath, "package");
    const release = {
      release_schema: 1,
      plugin_id: "example",
      plugin_version: "1.2.3",
      package_digest: packageDigest,
      artifact_digest: artifactDigest,
      artifact_url:
        `https://cowboy.example/plugin-artifacts/${packageValue}/example.cowboy-plugin`,
      publisher: "cowboy-first-party",
      signature: "signed",
      runtime_artifacts: [],
    };
    await Deno.writeTextFile(releasePath, JSON.stringify(release));
    await Deno.writeTextFile(
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
    await Deno.writeTextFile(publishedPackage, "tampered");
    const [tampered] = await checkProviderReleaseCoverage(plugins, catalog);
    assertEquals(tampered?.covered, false);
    assertEquals(
      tampered?.detail,
      `published artifact digest mismatch: ${publishedPackage}`,
    );
    await Deno.writeTextFile(publishedPackage, "package");

    await Deno.remove(releasePath);
    assertEquals(await checkProviderReleaseCoverage(plugins, catalog), [{
      plugin_id: "example",
      plugin_version: "1.2.3",
      covered: false,
      detail: "no exact signed release is published",
    }]);
  } finally {
    await Deno.remove(root, { recursive: true });
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
