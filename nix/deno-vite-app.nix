{ pkgs, lib, deno }:

# Build a Deno + Vite application in two layers. The fixed-output derivation
# contains dependency cache bytes only; the ordinary derivation owns source and
# always rebuilds when Cowboy changes. App-shell components are committed in
# Cowboy, so this builder deliberately has no cross-repository staging seam.
{
  pname,
  version ? "0.1.0",
  src,
  webRoot ? "web",
  depsHash,
  localPackages ? [ ],
  installArgs ? "--frozen",
  nodejs ? pkgs.nodejs_24,
}:
let
  webDeps = pkgs.stdenvNoCC.mkDerivation {
    pname = "${pname}-web-deps";
    inherit version;
    src = pkgs.runCommandLocal "${pname}-web-deps-src" { } ''
      mkdir -p $out/${webRoot}
      for f in deno.json deno.jsonc deno.lock package.json; do
        if [ -e "${src}/${webRoot}/$f" ]; then
          cp "${src}/${webRoot}/$f" "$out/${webRoot}/$f"
        fi
      done
      ${lib.concatMapStringsSep "\n" (package: ''
        mkdir -p "$out/$(dirname '${package}')"
        cp -R "${src}/${package}" "$out/${package}"
      '') localPackages}
    '';
    nativeBuildInputs = [ deno nodejs ];
    dontUnpack = true;
    dontConfigure = true;
    buildPhase = ''
      export HOME=$TMPDIR
      export DENO_DIR=$out
      export SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt
      cp -RL $src/. .
      chmod -R u+w .
      cd ${webRoot}
      deno install ${installArgs}
    '';
    dontInstall = true;
    dontFixup = true;
    outputHashMode = "recursive";
    outputHashAlgo = "sha256";
    outputHash = depsHash;
  };
in
pkgs.stdenvNoCC.mkDerivation {
  pname = "${pname}-web";
  inherit version src;
  nativeBuildInputs = [ deno nodejs ];
  dontConfigure = true;
  buildPhase = ''
    export HOME=$TMPDIR
    export DENO_DIR=$TMPDIR/deno-cache
    cp -R ${webDeps} $DENO_DIR
    chmod -R u+w $DENO_DIR
    cd ${webRoot}
    deno install ${installArgs}
    deno task build
  '';
  installPhase = ''
    cp -R dist $out
  '';
  dontFixup = true;
}
