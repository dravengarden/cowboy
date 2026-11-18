import { expect, test } from "bun:test";
import { vendorHosted } from "./vendor-runtime-artifacts.ts";

const url = "https://vendor.example/cli/1.0.0-1/linux-x64/cli.tar.gz";
const sha256 = "a".repeat(64);
const lock = {
  components: {
    "vendor-cli": {
      kind: "vendor_archive",
      version: "1.0.0",
      targets: { "linux-x86_64": { url, sha256 } },
    },
    "npm-cli": { kind: "npm_archive", version: "2.0.0" },
  },
};
const linux = { os: "linux", architecture: "x86_64" };
const component = {
  dependency: "vendor-cli",
  version: "1.0.0",
  command: "cli",
  artifact_url: url,
  artifact_digest: `sha256:${sha256}`,
};

test("an exact vendor lock match is not republished", () => {
  expect(vendorHosted(linux, component, lock)).toBe(true);
});

test("non-vendor components stay on the Catalog route", () => {
  expect(vendorHosted(linux, { ...component, dependency: "npm-cli" }, lock))
    .toBe(false);
});

test("a vendor component that drifts from the lock is refused", () => {
  for (
    const drifted of [
      { ...component, artifact_url: `${url}?x` },
      { ...component, artifact_digest: `sha256:${"b".repeat(64)}` },
      { ...component, version: "1.0.1" },
    ]
  ) {
    expect(() => vendorHosted(linux, drifted, lock)).toThrow();
  }
  expect(() =>
    vendorHosted({ os: "macos", architecture: "aarch64" }, component, lock)
  ).toThrow();
});
