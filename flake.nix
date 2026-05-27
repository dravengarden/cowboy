{
  description = "cowboy — drive coding-agent CLIs from anywhere over ACP, with one shared live progress";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };

      # Pure-Rust build of the v0 daemon. No frontend bundle yet (the UI is the
      # inline placeholder in server.rs), so this is a plain cargoLock build —
      # no bun/embed step. Add a vendored frontend derivation here once the
      # React UI lands and rust-embed pulls it in.
      cowboy = pkgs.rustPlatform.buildRustPackage {
        pname = "cowboy";
        version = "0.1.0";
        src = pkgs.lib.cleanSource ./.;
        cargoLock.lockFile = ./Cargo.lock;
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
