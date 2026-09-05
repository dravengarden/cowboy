/** Create-only publication: the hard link is the atomic commit point. */
export async function copyImmutable(
  source: string,
  destination: string,
  mode = 0o644,
): Promise<void> {
  await requireRegularFile(source);
  const expected = await sha256(source);
  await publish(destination, mode, async (temporary) => {
    await Deno.copyFile(source, temporary);
    if (await sha256(temporary) !== expected) {
      throw new Error(`publication source changed: ${source}`);
    }
  }, async () => await sha256(destination) === expected);
}

export async function copyImmutableText(
  value: string,
  destination: string,
  mode = 0o644,
): Promise<void> {
  await publish(destination, mode, async (temporary) => {
    await Deno.writeTextFile(temporary, value);
  }, async () => await Deno.readTextFile(destination) === value);
}

async function publish(
  destination: string,
  mode: number,
  prepare: (temporary: string) => Promise<void>,
  matches: () => Promise<boolean>,
): Promise<void> {
  if (!destination.startsWith("/")) {
    throw new Error("immutable publication target must be absolute");
  }
  const verifyExisting = async () => {
    await requireRegularFile(destination);
    if (!await matches()) {
      throw new Error(
        `immutable publication target already has different bytes: ${destination}`,
      );
    }
  };
  const directory = destination.slice(0, destination.lastIndexOf("/"));
  await Deno.mkdir(directory, { recursive: true });
  if (await exists(destination)) {
    await verifyExisting();
    return;
  }
  const temporary = await Deno.makeTempFile({
    dir: directory,
    prefix: ".cowboy-publication-",
  });
  try {
    await prepare(temporary);
    await Deno.chmod(temporary, mode);
    try {
      // rename replaces an existing destination on POSIX. link never does.
      await Deno.link(temporary, destination);
    } catch (error) {
      if (!(error instanceof Deno.errors.AlreadyExists)) throw error;
      await verifyExisting();
    }
  } finally {
    await Deno.remove(temporary);
  }
}

async function requireRegularFile(path: string): Promise<void> {
  const info = await Deno.lstat(path);
  if (!info.isFile || info.isSymlink) {
    throw new Error(`immutable publication requires a regular file: ${path}`);
  }
}

export async function sha256(path: string): Promise<string> {
  await requireRegularFile(path);
  const output = await new Deno.Command("sha256sum", {
    args: [path],
    clearEnv: true,
  }).output();
  if (!output.success) throw new Error(`sha256sum failed for ${path}`);
  return new TextDecoder().decode(output.stdout).trim().split(/\s+/)[0]
    .toLowerCase();
}

export async function exists(path: string): Promise<boolean> {
  try {
    await Deno.lstat(path);
    return true;
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return false;
    throw error;
  }
}
