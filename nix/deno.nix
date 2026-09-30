{ pkgs }:

# Keep Cowboy's Web toolchain hermetic without importing another repository.
# Delete this derivation once nixpkgs supplies the exact supported Deno release.
pkgs.stdenvNoCC.mkDerivation rec {
  pname = "deno";
  version = "2.9.7";
  src = pkgs.fetchurl {
    url = "https://github.com/denoland/deno/releases/download/v${version}/deno-x86_64-unknown-linux-gnu.zip";
    hash = "sha256-xlJ/JPSxYDHTrk+p9ljV8RU0yNhM59yFAkICgJGcNJA=";
  };
  nativeBuildInputs = [ pkgs.unzip pkgs.autoPatchelfHook ];
  buildInputs = [ pkgs.stdenv.cc.cc.lib pkgs.zlib ];
  unpackPhase = "unzip $src";
  installPhase = "install -Dm755 deno $out/bin/deno";
  meta = {
    description = "A modern runtime for JavaScript and TypeScript";
    homepage = "https://deno.land/";
    mainProgram = "deno";
  };
}
