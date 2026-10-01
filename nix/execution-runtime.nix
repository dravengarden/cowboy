{ pkgs }:
let
  lock = builtins.fromJSON (builtins.readFile ../components/execution-runtime/lock.json);
  source = pkgs.fetchurl {
    url = lock.url;
    hash = lock.archive_hash;
  };
in
assert lock.schema == 1 && lock.protocol == 1 && lock.system == pkgs.stdenv.hostPlatform.system;
pkgs.runCommand "cowboy-execution-runtime-${lock.version}"
  {
    passthru = {
      executorVersion = lock.version;
      executorDigest = lock.executable_sha256;
    };
  }
  ''
    mkdir unpacked "$out"
    tar -xzf ${source} -C unpacked
    cp -R unpacked/${lock.directory}/. "$out/"
    printf '%s  %s\n' ${lock.executable_sha256} "$out/bin/codex" | sha256sum -c -
    printf '%s  %s\n' ${lock.metadata_sha256} "$out/codex-package.json" | sha256sum -c -
    test "$("$out/bin/codex" --version)" = "codex-cli ${lock.version}"
  ''
