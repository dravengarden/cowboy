{
  description = "cowboy — drive coding-agent CLIs from anywhere over ACP, with one shared live progress";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  # Shared front-end SDK + the shared Nix builders (buildDenoViteApp, the pinned
  # deno) from the public shared-utils monorepo. The SPA is built via that shared
  # builder — NOT a hand-rolled FOD here — so source changes always rebuild and
  # the deno pin / _shell staging are not copy-pasted into this flake.
  inputs.shared-utils.url = "github:dravengarden/shared-utils";
  inputs.shared-utils.inputs.nixpkgs.follows = "nixpkgs";

  outputs = { self, nixpkgs, shared-utils }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      shared = shared-utils.lib.${system};

      # The SPA, built through the shared, footgun-free builder: a deps-only FOD
      # (vendored npm cache, keyed by the lockfiles → depsHash below) + a normal
      # content-addressed offline build. Any source edit rebuilds automatically;
      # only refresh depsHash when web/deno.lock or web/package.json change
      # (lib.fakeHash → build → copy "got"). The builder also stages the
      # @shared-utils/ui SDK into web/src/_shell.
      cowboy-web = shared.buildDenoViteApp {
        pname = "cowboy";
        version = "0.1.0";
        src = pkgs.lib.cleanSource ./.;
        depsHash = "sha256-48jLO1HPzxqCjKcqq0sJ6ToqX7sGj1HQsJxnbFStTP4=";
      };

      # The Rust binary, embedding the built SPA via rust-embed
      # (`#[folder = "web/dist"]`). `preBuild` drops the builder output where the
      # embed macro expects it before the release cargo build runs.
      cowboy = pkgs.rustPlatform.buildRustPackage {
        pname = "cowboy";
        version = "0.1.0";
        src = pkgs.lib.cleanSource ./.;
        # fetchCargoVendor (cargoHash) rather than cargoLock: it downloads
        # crates via python-requests, which sends a User-Agent. crates.io now
        # 403s the download endpoint without one, and the plain-fetchurl
        # cargoLock path sends none. Refresh: lib.fakeHash → build → copy hash.
        cargoHash = "sha256-h36xRafN5a8ia+lxIV5ytZ0n0gvoIgBwAEnoefpw9eU=";
        # The memory store shells out to `git` (store::Store::commit/ensure_git_repo,
        # ported from mnemosyne). Its unit tests `git init` a temp store + commit
        # entirely offline, so `git` must be on the checkPhase PATH (the build
        # sandbox has none by default). HOME is set so git's local-config writes
        # don't fail looking for ~/.gitconfig.
        nativeBuildInputs = [ pkgs.git ];
        preCheck = ''
          export HOME=$(mktemp -d)
        '';
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
        # Rust toolchain + sccache compiler cache, plus the frontend toolchain
        # (the shared pinned deno 2.8.1 + node 24 for any node-shaped tool that
        # deno's npm interop can't shim).
        nativeBuildInputs = with pkgs; [
          rustc
          cargo
          clippy
          rustfmt
          sccache
          just
          nodejs_24
        ] ++ [ shared.deno ];

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
