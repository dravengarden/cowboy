{
  description = "cowboy — drive coding-agent CLIs from anywhere over ACP, with one shared live progress";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  # Shared front-end SDK (business- and portal-free React + MUI primitives) from
  # the public shared-utils monorepo, staged into the web build — NOT vendored
  # (web/src/_shell/ is gitignored, materialized at build / via the dev symlink).
  inputs.shared-utils.url = "github:dravengarden/shared-utils";
  inputs.shared-utils.inputs.nixpkgs.follows = "nixpkgs";

  outputs = { self, nixpkgs, shared-utils }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      sharedUiSrc = shared-utils.packages.${system}.ui;

      # Deno 2.8.1 — nixpkgs `nixos-unstable` only ships 2.7.14 today, so we
      # pull the official prebuilt binary directly. Standard pattern:
      # fetchurl → unzip → autoPatchelfHook fixes the dynamic linker. Once
      # nixpkgs catches up, this whole derivation can be deleted and the
      # uses below swap back to `pkgs.deno`.
      deno = pkgs.stdenvNoCC.mkDerivation rec {
        pname = "deno";
        version = "2.8.1";
        src = pkgs.fetchurl {
          url = "https://github.com/denoland/deno/releases/download/v${version}/deno-x86_64-unknown-linux-gnu.zip";
          hash = "sha256-LXu2GVImrIMuC/cQmhFfCvZe5prHl6S73lsnoGzCQtk=";
        };
        nativeBuildInputs = [ pkgs.unzip pkgs.autoPatchelfHook ];
        buildInputs = [ pkgs.stdenv.cc.cc.lib pkgs.zlib ];
        unpackPhase = "unzip $src";
        installPhase = ''
          install -Dm755 deno $out/bin/deno
        '';
        meta = {
          description = "A modern runtime for JavaScript and TypeScript";
          homepage = "https://deno.land/";
          mainProgram = "deno";
        };
      };

      # Step 1 — build the SPA, split so SOURCE CHANGES ALWAYS REBUILD.
      #
      # The bundle's bytes vary with source, so it must NOT be a fixed-output
      # derivation (FOD): an FOD is addressed by its declared `outputHash`, so
      # Nix reuses the cached output whenever that hash is unchanged — silently
      # embedding a STALE bundle when only the source moved. That is the "why
      # isn't the UI updating after a rebuild" footgun; it forced a manual hash
      # rebump on every single web edit (and bit us in prod).
      #
      # Fix = separate the two concerns the old single FOD conflated:
      #   web-deps   — a SMALL FOD that only vendors the npm deps (the part that
      #                needs network). Keyed by the lockfiles, so its hash changes
      #                ONLY when deno.lock / package.json change.
      #   cowboy-web — a NORMAL, sandboxed, content-addressed derivation that
      #                consumes that vendored cache OFFLINE. Any source edit → new
      #                inputs → automatic rebuild. No hash bump, ever.

      # Deps-only FOD: populate a relocatable DENO_DIR npm cache from the
      # lockfiles alone. Refresh the hash ONLY when deps change:
      # `lib.fakeHash` → build → copy the "got" hash back.
      web-deps = pkgs.stdenvNoCC.mkDerivation {
        pname = "cowboy-web-deps";
        version = "0.1.0";
        src = pkgs.runCommandLocal "cowboy-web-deps-src" { } ''
          mkdir -p $out
          cp ${./web/deno.json} $out/deno.json
          cp ${./web/deno.lock} $out/deno.lock
          cp ${./web/package.json} $out/package.json
        '';
        nativeBuildInputs = [ deno pkgs.nodejs_24 ];
        dontUnpack = true;
        dontConfigure = true;
        buildPhase = ''
          export HOME=$TMPDIR
          export DENO_DIR=$out
          cp -RL $src/. .
          chmod -R u+w .
          deno install
        '';
        dontInstall = true;
        dontFixup = true;
        outputHashMode = "recursive";
        outputHashAlgo = "sha256";
        outputHash = "sha256-kAJVt2SFRj4mzQsf6bUQtByqJ1GjsXliZ6uidAfa1V0=";
      };

      # Offline SPA build. NORMAL derivation (no outputHash) → rebuilds on any
      # source change. Stages the shared SDK + the vendored deno cache, then
      # builds with no network: a missing dep fails loudly instead of drifting.
      cowboy-web = pkgs.stdenvNoCC.mkDerivation {
        pname = "cowboy-web";
        version = "0.1.0";
        src = pkgs.lib.cleanSource ./.;
        nativeBuildInputs = [ deno pkgs.nodejs_24 ];
        dontConfigure = true;
        buildPhase = ''
          export HOME=$TMPDIR
          # DENO_DIR must be writable (deno touches it); copy the vendored cache.
          export DENO_DIR=$TMPDIR/deno-cache
          cp -R ${web-deps} $DENO_DIR
          chmod -R u+w $DENO_DIR
          # Stage the shared SDK from the shared-utils ui package.
          mkdir -p web/src/_shell
          cp ${sharedUiSrc}/* web/src/_shell/
          chmod -R u+w web/src/_shell
          cd web
          # --frozen: never mutate the lock here (deps are pre-vendored); the
          # sandbox has no network, so this only re-materializes node_modules
          # from the cached tarballs.
          deno install --frozen
          deno task build
        '';
        installPhase = ''
          cp -R dist $out
        '';
        dontFixup = true;
      };

      # Step 2 — the Rust binary, embedding the built SPA via rust-embed
      # (`#[folder = "web/dist"]`). `preBuild` drops the FOD output where the
      # embed macro expects it before the release cargo build runs.
      cowboy = pkgs.rustPlatform.buildRustPackage {
        pname = "cowboy";
        version = "0.1.0";
        src = pkgs.lib.cleanSource ./.;
        # fetchCargoVendor (cargoHash) rather than cargoLock: it downloads
        # crates via python-requests, which sends a User-Agent. crates.io now
        # 403s the download endpoint without one, and the plain-fetchurl
        # cargoLock path sends none. Refresh: lib.fakeHash → build → copy hash.
        cargoHash = "sha256-0AyXaJGw03g5A03T3P6Zoj5OdrV9XWTAL95vhRSvjbw=";
        preBuild = ''
          mkdir -p web/dist
          cp -R ${cowboy-web}/. web/dist/
        '';
        meta = {
          description = "Drive coding-agent CLIs from anywhere over ACP";
          mainProgram = "cowboy";
        };
      };
    in
    {
      packages.${system} = {
        default = cowboy;
        cowboy = cowboy;
        cowboy-web = cowboy-web;
        deno = deno;
      };

      devShells.${system}.default = pkgs.mkShell {
        # Rust toolchain + sccache compiler cache, plus the frontend toolchain
        # (deno 2.8.1 binary override + node 24 for any node-shaped tool
        # that deno's npm interop can't shim).
        nativeBuildInputs = with pkgs; [
          rustc
          cargo
          clippy
          rustfmt
          sccache
          just
          deno
          nodejs_24
        ];

        # All Rust builds in this project go through sccache (see design.md §10).
        # CARGO_INCREMENTAL=0 because incremental compilation and sccache
        # conflict; disabling it maximizes cache hits.
        RUSTC_WRAPPER = "sccache";
        CARGO_INCREMENTAL = "0";

        shellHook = ''
          echo "cowboy dev shell — rust + sccache + deno"
          sccache --version >/dev/null 2>&1 && echo "sccache: $(sccache --version)"
          deno --version 2>/dev/null | head -1
        '';
      };
    };
}
