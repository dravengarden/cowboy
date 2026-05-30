{
  description = "cowboy — drive coding-agent CLIs from anywhere over ACP, with one shared live progress";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };

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

      # Step 1 — build the SPA. Fixed-output derivation because
      # `deno install` needs network; the hash only changes when the bundle
      # changes. Output is the built `web/dist`. Refresh the hash with
      # `lib.fakeHash` → build → copy the "got" hash back.
      cowboy-web = pkgs.stdenvNoCC.mkDerivation {
        pname = "cowboy-web";
        version = "0.1.0";
        src = pkgs.lib.cleanSource ./.;
        nativeBuildInputs = [ deno pkgs.nodejs_24 ];
        dontConfigure = true;
        buildPhase = ''
          export HOME=$TMPDIR
          export DENO_DIR=$TMPDIR/deno-cache
          cd web
          # No --frozen here: the FOD's outputHash already pins the bundle
          # bit-for-bit, and the lockfile lives in-tree so an in-source
          # update is what we want when a dep is bumped.
          deno install
          deno task build
        '';
        installPhase = ''
          cp -R dist $out
        '';
        dontFixup = true;
        outputHashMode = "recursive";
        outputHashAlgo = "sha256";
        outputHash = "sha256-pdRPLpufRIu+ttobEmFnRVT2ELfZU5wvYP6FjS8TQ6M=";
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
        # Build id the binary serves at /version.json for the atlantis portal's
        # update-banner poll. The app's commit SHA changes every deploy; a dirty
        # tree (local `nix build`) has no rev, so fall back to the static
        # version. Read via option_env! in src/server.rs.
        ATLANTIS_BUILD_VERSION = self.shortRev or "0.1.0";
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
