{
  description = "cowboy — drive coding-agent CLIs from anywhere over ACP, with one shared live progress";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  # Shared front-end SDK + the shared Nix builders (buildDenoViteApp, the pinned
  # deno) from the public shared-utils monorepo. The SPA is built via that shared
  # builder — NOT a hand-rolled FOD here — so source changes always rebuild and
  # the deno pin / _shell staging are not copy-pasted into this flake.
  inputs.shared-utils.url =
    "git+ssh://git@github.com/dravengarden/shared-utils.git?ref=refs/heads/main";
  inputs.shared-utils.inputs.nixpkgs.follows = "nixpkgs";

  outputs = { self, nixpkgs, shared-utils }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      shared = shared-utils.lib.${system};

      # Backend and frontend are independent deployment artifacts. Keep this
      # closure explicit: docs, Web, native-shell, and operational edits must
      # not change the Rust package's store path and restart the API unit.
      # protocol.ts is the sole frontend input because Rust contract tests
      # deliberately compile-check its wire tags.
      cowboy-src = pkgs.lib.fileset.toSource {
        root = ./.;
        fileset = pkgs.lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
          ./migrations
          ./web/src/protocol.ts
        ];
      };

      # Agentd has a deliberately tiny source closure and is packaged
      # separately. Ordinary API, SPA, ACP, or worker edits therefore leave
      # its ExecStart path unchanged.
      agentd-src = pkgs.lib.fileset.toSource {
        root = ./.;
        fileset = pkgs.lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src/lib.rs
          ./src/agentd.rs
          ./src/runtime_wire.rs
          ./src/bin/cowboy-agentd.rs
        ];
      };

      # Only behavior that runs inside a detached session contributes to the
      # pool generation. A control-plane-only change updates Cowboy without
      # draining live ACP sessions.
      worker-generation-files = [
        ./Cargo.toml
        ./Cargo.lock
        ./worker-generation.txt
        ./src/acp.rs
        ./src/agent_model.rs
        ./src/agent_sink.rs
        ./src/cgroup.rs
        ./src/provider/mod.rs
        ./src/runtime_wire.rs
        ./src/worker.rs
        ./src/bin/cowboy-acp-worker.rs
      ];
      worker-generation = "worker-" + builtins.substring 0 20 (
        builtins.hashString "sha256" (
          pkgs.lib.concatMapStringsSep ":"
            (path: builtins.hashFile "sha256" path)
            worker-generation-files
        )
      );

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
        depsHash = "sha256-PutelqKh3fSO/fxmlKxg19fupluO1QwsrEnl19CAa+E=";
      };

      # This host's pinned Nixpkgs still has the first fetchCargoVendor
      # implementation, which downloads through crates.io's rate-limited API.
      # crates.io now rejects that bulk endpoint with a data-access 403. Newer
      # Nixpkgs uses the official immutable static CDN for exactly this reason.
      # Patch only the vendoring helper inside the FOD; Cargo.lock checksums and
      # the aggregate cargo hash remain fully enforced.
      cowboy-cargo-deps = pkgs.rustPlatform.fetchCargoVendor {
        pname = "cowboy";
        version = "0.1.0";
        src = cowboy-src;
        hash = "sha256-gLDf/3iWcRFGbgxznXSo7azZoAxgdCBYAZzfX68gvz8=";
        preBuild = ''
          vendor_util="$(command -v fetch-cargo-vendor-util-v2 || command -v fetch-cargo-vendor-util)"
          if grep -q "https://crates.io/api/v1/crates/" "$vendor_util"; then
            patched_util="$TMPDIR/cargo-vendor-bin/$(basename "$vendor_util")"
            mkdir -p "$(dirname "$patched_util")"
            cp "$vendor_util" "$patched_util"
            chmod u+w "$patched_util"
            substituteInPlace "$patched_util" \
              --replace-fail \
                "https://crates.io/api/v1/crates/" \
                "https://static.crates.io/crates/"
            export PATH="$(dirname "$patched_util"):$PATH"
          fi
        '';
      };

      # API/control plane + detached ACP worker. The SPA is served from a
      # runtime path and is intentionally absent from this derivation.
      cowboy = pkgs.rustPlatform.buildRustPackage {
        pname = "cowboy";
        version = "0.1.0";
        src = cowboy-src;
        cargoDeps = cowboy-cargo-deps;
        cargoBuildFlags = [ "--bin" "cowboy" "--bin" "cowboy-acp-worker" ];
        nativeCheckInputs = [ pkgs.gitMinimal ];
        passthru.workerGeneration = worker-generation;
        meta = {
          description = "Drive coding-agent CLIs from anywhere over ACP";
          mainProgram = "cowboy";
        };
      };

      cowboy-agentd = pkgs.rustPlatform.buildRustPackage {
        pname = "cowboy-agentd";
        version = "0.1.0";
        src = agentd-src;
        cargoDeps = cowboy-cargo-deps;
        cargoBuildFlags = [ "--no-default-features" "--bin" "cowboy-agentd" ];
        doCheck = false;
        meta = {
          description = "Stable local broker for detached Cowboy ACP workers";
          mainProgram = "cowboy-agentd";
        };
      };

      cowboy-source-boundary = pkgs.runCommand "cowboy-source-boundary" { } ''
        test ! -e ${cowboy-src}/docs
        test ! -e ${cowboy-src}/web/public
        test -e ${cowboy-src}/web/src/protocol.ts
        touch "$out"
      '';
    in
    {
      packages.${system} = {
        default = cowboy;
        cowboy = cowboy;
        cowboy-agentd = cowboy-agentd;
        cowboy-web = cowboy-web;
      };

      # `cowboy`'s buildRustPackage check phase runs the Rust tests; cowboy-web's
      # build runs TypeScript checking before Vite. Developer lint/test policy is
      # additionally enforced by `just check` in CI.
      checks.${system} = {
        inherit cowboy cowboy-agentd cowboy-source-boundary cowboy-web;
      };

      devShells.${system}.default = pkgs.mkShell {
        # Rust toolchain plus opt-in sccache, and the frontend toolchain
        # (the shared pinned deno 2.8.1 + node 24 for any node-shaped tool that
        # deno's npm interop can't shim).
        nativeBuildInputs = with pkgs; [
          rustc
          cargo
          clippy
          rustfmt
          sccache
          cargo-nextest
          cargo-deny
          cargo-machete
          cargo-sweep
          just
          go
          nodejs_24
        ] ++ [ shared.deno ];

        shellHook = ''
          echo "cowboy dev shell — rust + optional sccache + deno"
          sccache --version >/dev/null 2>&1 && echo "sccache: $(sccache --version)"
          deno --version 2>/dev/null | head -1
        '';
      };
    };
}
