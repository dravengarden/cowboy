{ pkgs }:

# Core build tooling has its own pin. Agent Plugins select their private Node
# through provider-runtime/lock.json and never inherit this executable.
pkgs.stdenvNoCC.mkDerivation rec {
  pname = "nodejs";
  version = "24.21.0";
  src = pkgs.fetchurl {
    url = "https://nodejs.org/dist/v${version}/node-v${version}-linux-x64.tar.xz";
    sha256 = "fd8e59d5a511510f6a298afb548f18c7d2b1be404d8b4a27d94fbe49f56cb2d6";
  };
  nativeBuildInputs = [ pkgs.autoPatchelfHook ];
  buildInputs = [ pkgs.stdenv.cc.cc.lib ];
  dontConfigure = true;
  dontBuild = true;
  installPhase = ''
    mkdir -p "$out"
    cp -R bin include lib share "$out/"
    for cli in npm-cli npx-cli; do
      substituteInPlace "$out/lib/node_modules/npm/bin/$cli.js" \
        --replace-fail '#!/usr/bin/env node' "#!$out/bin/node"
    done
  '';
  meta = {
    description = "Node.js LTS for Cowboy build tooling";
    homepage = "https://nodejs.org/";
    license = pkgs.lib.licenses.mit;
    platforms = [ "x86_64-linux" ];
    mainProgram = "node";
  };
}
