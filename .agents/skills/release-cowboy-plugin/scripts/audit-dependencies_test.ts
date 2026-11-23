import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import {
  auditDependencies,
  classifySource,
  discoverProviderIds,
  type FetchFn,
} from "./audit-dependencies.ts";

test("all audit discovers only directories with Provider manifests", async () => {
  const root = await mkdtemp(join(tmpdir(), "cowboy-provider-audit-"));
  try {
    for (const name of ["grok", "claude-code", "runtime-packages"]) {
      await mkdir(`${root}/${name}`);
    }
    await writeFile(`${root}/grok/provider.json`, "{}");
    await writeFile(`${root}/claude-code/provider.json`, "{}");
    await writeFile(`${root}/README.md`, "not a Provider");

    assertEquals(discoverProviderIds(new URL(`file://${root}/`)), ["claude-code", "grok"]);
  } finally {
    await rm(root, { recursive: true });
  }
});

test("classifySource identifies npm, antigravity CLI, workspace-local and git pins", () => {
  assertEquals(classifySource("https://registry.npmjs.org/@openai/codex/-/codex-0.171.0.tgz"), {
    kind: "npm",
    packageName: "@openai/codex",
  });
  assertEquals(
    classifySource(
      "https://storage.googleapis.com/antigravity-public/antigravity-cli/1.5.14-530269/",
    ),
    { kind: "antigravity_cli" },
  );
  assertEquals(classifySource("cowboy:components/provider-runtime/packages/antigravity-acp"), {
    kind: "workspace_local",
    relativePath: "components/provider-runtime/packages/antigravity-acp",
  });
  assertEquals(
    classifySource(
      "git+ssh://git@github.com/dravengarden/columbus.git#920a18efe145:projects/claude-deepseek",
    ),
    {
      kind: "git_pinned",
      repo: "git+ssh://git@github.com/dravengarden/columbus.git",
      rev: "920a18efe145",
      subdir: "projects/claude-deepseek",
    },
  );
});

test("auditDependencies checks npm, Antigravity GitHub releases, workspace drift and git pins", async () => {
  const root = await mkdtemp(join(tmpdir(), "cowboy-audit-deps-"));
  try {
    await mkdir(join(root, "plugins/antigravity"), { recursive: true });
    await mkdir(join(root, "plugins/codex"), { recursive: true });
    await mkdir(join(root, "components/pkg-ok"), { recursive: true });
    await mkdir(join(root, "components/pkg-drift"), { recursive: true });
    await writeFile(
      join(root, "components/pkg-ok/package.json"),
      JSON.stringify({ version: "1.0.0" }),
    );
    await writeFile(
      join(root, "components/pkg-drift/package.json"),
      JSON.stringify({ version: "1.0.2" }),
    );

    await writeFile(
      join(root, "plugins/antigravity/provider.json"),
      JSON.stringify({
        id: "antigravity",
        version: "0.2.27",
        runtime: {
          dependencies: [
            {
              id: "local-ok",
              version: "1.0.0",
              source: "cowboy:components/pkg-ok",
              integrity: "sha256:aaa",
              private: true,
            },
            {
              id: "local-drift",
              version: "1.0.1",
              source: "cowboy:components/pkg-drift",
              integrity: "sha256:bbb",
              private: true,
            },
            {
              id: "google-antigravity-cli",
              version: "1.5.14",
              source:
                "https://storage.googleapis.com/antigravity-public/antigravity-cli/1.5.14-123/",
              integrity: "sha256:ccc",
              private: true,
            },
          ],
        },
      }),
    );

    await writeFile(
      join(root, "plugins/codex/provider.json"),
      JSON.stringify({
        id: "codex",
        version: "3.6.32",
        runtime: {
          dependencies: [
            {
              id: "openai-codex",
              version: "0.171.0",
              source: "https://registry.npmjs.org/@openai/codex/-/codex-0.171.0.tgz",
              integrity: "sha512-ok",
              private: true,
            },
            {
              id: "gateway",
              version: "0.1.0",
              source: "git+ssh://git@github.com/dravengarden/columbus.git#abcdef123456:projects/gw",
              integrity: "sha256:ddd",
              private: true,
            },
          ],
        },
      }),
    );

    const mockFetch: FetchFn = async (url) => {
      if (url.includes("google-antigravity/antigravity-cli")) {
        return new Response(
          JSON.stringify({ tag_name: "v1.5.15", html_url: "https://github.com/example/1.5.15" }),
          { status: 200 },
        );
      }
      if (url.endsWith("/0.171.0")) {
        return new Response(
          JSON.stringify({
            version: "0.171.0",
            dist: {
              integrity: "sha512-ok",
              tarball: "https://registry.npmjs.org/@openai/codex/-/codex-0.171.0.tgz",
            },
          }),
          { status: 200 },
        );
      }
      if (url.endsWith("/latest")) {
        return new Response(
          JSON.stringify({
            version: "0.172.0",
            dist: { integrity: "sha512-new" },
          }),
          { status: 200 },
        );
      }
      return new Response("not found", { status: 404 });
    };

    const report = await auditDependencies({ repoRoot: root, fetchFn: mockFetch });
    const byId = Object.fromEntries(report.dependencies.map((d) => [d.dependency_id, d]));
    assertEquals(byId["local-ok"]?.status, "workspace_local");
    assertEquals(byId["local-drift"]?.status, "workspace_drift");
    assertEquals(byId["local-drift"]?.candidate_version, "1.0.2");
    assertEquals(byId["google-antigravity-cli"]?.status, "upgrade_available");
    assertEquals(byId["google-antigravity-cli"]?.candidate_version, "1.5.15");
    assertEquals(byId["openai-codex"]?.status, "upgrade_available");
    assertEquals(byId["openai-codex"]?.candidate_version, "0.172.0");
    assertEquals(byId.gateway?.status, "git_pinned");
  } finally {
    await rm(root, { recursive: true });
  }
});
