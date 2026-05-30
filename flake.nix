{
  description = "cowboy — drive coding-agent CLIs from anywhere over ACP, with one shared live progress";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };

      # Step 1 — build the SPA (omega's recipe). Fixed-output derivation
      # because `bun install` needs network; the hash only changes when the
      # bundle changes. Output is the built `web/dist`. Refresh the hash with
      # `lib.fakeHash` → build → copy the "got" hash back.
      cowboy-web = pkgs.stdenvNoCC.mkDerivation {
        pname = "cowboy-web";
        version = "0.1.0";
        src = pkgs.lib.cleanSource ./.;
        nativeBuildInputs = [ pkgs.bun pkgs.nodejs_22 ];
        dontConfigure = true;
        buildPhase = ''
          export HOME=$TMPDIR
          cd web
          bun install --frozen-lockfile --no-progress
          node node_modules/.bin/vite build
        '';
        installPhase = ''
          cp -R dist $out
        '';
        dontFixup = true;
        outputHashMode = "recursive";
        outputHashAlgo = "sha256";
        outputHash = "sha256-umg41293Hl75qvS1N6OqmP0iKe9xGnTbq+a927TDKTk=";
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
        cargoHash = "sha256-KN9wGQeIlP4vyNBsKvORlaLmDK35vbe7Hgl77MgsF6I=";
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
      };

      devShells.${system}.default = pkgs.mkShell {
        # Rust toolchain + sccache compiler cache, plus the frontend toolchain.
        nativeBuildInputs = with pkgs; [
          rustc
          cargo
          clippy
          rustfmt
          sccache
          just
          bun
          nodejs_22
        ];

        # All Rust builds in this project go through sccache (see design.md §10).
        # CARGO_INCREMENTAL=0 because incremental compilation and sccache
        # conflict; disabling it maximizes cache hits.
        RUSTC_WRAPPER = "sccache";
        CARGO_INCREMENTAL = "0";

        shellHook = ''
          echo "cowboy dev shell — rust + sccache + bun"
          sccache --version >/dev/null 2>&1 && echo "sccache: $(sccache --version)"
        '';
      };
    };
}
