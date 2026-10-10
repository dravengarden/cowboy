{ pkgs, lib, bun }:

# Build a Bun + Vite application in two layers. The fixed-output derivation
# contains installed dependency bytes only; the ordinary derivation owns source
# and always rebuilds when Cowboy changes. App-shell components are committed in
# Cowboy, so this builder deliberately has no cross-repository staging seam.
{
  pname,
  version ? "0.1.0",
  src,
  webRoot ? "web",
  depsHash,
  # Every member of the repository's Bun workspace, the application included.
  # Only the manifests enter the dependency derivation; Bun links each member
  # to its source directory and gives it a node_modules of links into the
  # workspace store.
  workspaces,
  # Lifecycle scripts stay off: they would run with network inside the
  # fixed-output derivation, and their output is not part of the lockfile's
  # identity.
  installArgs ? "--frozen-lockfile --ignore-scripts",
}:
let
  eachWorkspace = lib.concatMapStringsSep "\n";
  webDeps = pkgs.stdenvNoCC.mkDerivation {
    pname = "${pname}-web-deps";
    inherit version;
    src = pkgs.runCommandLocal "${pname}-web-deps-src" { } ''
      mkdir -p $out
      for f in package.json bun.lock bunfig.toml; do
        if [ -e "${src}/$f" ]; then cp "${src}/$f" "$out/$f"; fi
      done
      ${eachWorkspace (member: ''
        mkdir -p "$out/${member}"
        cp "${src}/${member}/package.json" "$out/${member}/package.json"
      '') workspaces}
    '';
    nativeBuildInputs = [ bun ];
    dontUnpack = true;
    dontConfigure = true;
    buildPhase = ''
      export HOME=$TMPDIR
      export BUN_INSTALL_CACHE_DIR=$TMPDIR/bun-cache
      export SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt
      cp -RL $src/. .
      chmod -R u+w .
      # Plain copies keep the tree free of hardlinks into the discarded cache.
      bun install ${installArgs} --backend=copyfile --no-progress
      mkdir -p $out
      cp -R node_modules $out/node_modules
      ${eachWorkspace (member: ''
        if [ -d "${member}/node_modules" ]; then
          mkdir -p "$out/${member}"
          cp -R "${member}/node_modules" "$out/${member}/node_modules"
        fi
      '') workspaces}
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
  nativeBuildInputs = [ bun ];
  dontConfigure = true;
  buildPhase = ''
    export HOME=$TMPDIR
    # Offline: dependencies are pre-installed and the sandbox has no network,
    # so a dependency missing from the lockfile fails loudly instead of
    # silently drifting. The links between the trees are relative.
    cp -R ${webDeps}/. .
    chmod -R u+w node_modules
    # Package bins carry a `/usr/bin/env node` shebang, and the sandbox has
    # neither /usr/bin/env nor node. Point them at the pinned Bun under node's
    # name, which is how Bun selects its node mode.
    mkdir -p $TMPDIR/bin
    ln -s ${bun}/bin/bun $TMPDIR/bin/node
    export PATH=$TMPDIR/bin:$PATH
    patchShebangs node_modules
    cd ${webRoot}
    bun run build
  '';
  installPhase = ''
    cp -R dist $out
  '';
  dontFixup = true;
}
