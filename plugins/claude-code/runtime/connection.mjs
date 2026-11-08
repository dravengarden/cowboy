import { lstat, readFile } from "node:fs/promises";
import { isAbsolute } from "node:path";

export const EXECUTOR_DIGEST =
  "sha256:8bf204b36a2f6dd0dab73aa2f639892e67ef9ac8befccb4a05b1496ebf25c479";

export async function readDescriptor(path) {
  if (!isAbsolute(path)) {
    throw new Error("Execution descriptor must be absolute");
  }
  const stat = await lstat(path);
  if (!stat.isFile() || stat.size > 32768 || stat.mode & 0o077) {
    throw new Error("Execution descriptor must be a private regular file");
  }
  const descriptor = JSON.parse(await readFile(path, "utf8"));
  const url = new URL(descriptor.endpoint);
  const binding = descriptor.binding;
  if (
    descriptor.schema !== 1 ||
    binding?.schema !== 1 ||
    binding.environment?.protocol !== 1 ||
    binding.environment.executor_digest !== EXECUTOR_DIGEST ||
    !/^[a-zA-Z0-9_-]{1,128}$/.test(binding.environment.id) ||
    !isAbsolute(binding.workspace?.cwd ?? "") ||
    !/^[a-f0-9]{64}$/.test(descriptor.bearer_token) ||
    url.protocol !== "ws:" ||
    url.hostname !== "127.0.0.1" ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    url.pathname !== "/"
  )
    throw new Error("Unsupported execution descriptor");
  return descriptor;
}

export { Connection } from "../../../components/provider-runtime/execution-connection.mjs";
