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

      # Machine has a deliberately tiny source closure and is packaged
      # separately. Ordinary API, SPA, ACP, or worker edits therefore leave
      # its ExecStart path unchanged.
      machine-src = pkgs.lib.fileset.toSource {
        root = ./.;
        fileset = pkgs.lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src/lib.rs
          ./src/machine_broker.rs
          ./src/machine_cli.rs
          ./src/machine_auth.rs
          ./src/machine_components.rs
          ./src/machine_install.rs
          ./src/machine_protocol.rs
          ./src/runtime_wire.rs
          ./src/bin/cowboy-machine-install.rs
          ./src/bin/cowboy-machine.rs
        ];
      };

      code-adapter-src = pkgs.lib.fileset.toSource {
        root = ./.;
        fileset = pkgs.lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src/lib.rs
          ./src/code_adapter.rs
          ./src/code_review.rs
          ./src/files.rs
          ./src/bin/cowboy-code-adapter.rs
        ];
      };

      zed-adapter-src = pkgs.lib.cleanSource ./zed-adapter;

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
        hash = "sha256-7RpCt0wtMNE6GvXFRFn/4PabLuCQi3+aLTtI4pRlq3Y=";
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
        nativeCheckInputs = [ pkgs.cacert pkgs.gitMinimal pkgs.openssh ];
        preCheck = ''
          export SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt
        '';
        passthru.workerGeneration = worker-generation;
        meta = {
          description = "Drive coding-agent CLIs from anywhere over ACP";
          mainProgram = "cowboy";
        };
      };

      cowboy-machine = pkgs.rustPlatform.buildRustPackage {
        pname = "cowboy-machine";
        version = "0.1.0";
        src = machine-src;
        cargoDeps = cowboy-cargo-deps;
        cargoBuildFlags = [
          "--no-default-features"
          "--features"
          "machine-host"
          "--bin"
          "cowboy-machine"
          "--bin"
          "cowboy-machine-install"
        ];
        nativeBuildInputs = [ pkgs.makeWrapper ];
        postInstall = ''
          wrapProgram $out/bin/cowboy-machine \
            --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.openssh ]}
        '';
        doCheck = false;
        meta = {
          description = "Stable Cowboy Machine host for detached ACP workers";
          mainProgram = "cowboy-machine";
        };
      };

      cowboy-code-adapter = pkgs.rustPlatform.buildRustPackage {
        pname = "cowboy-code-adapter";
        version = "0.1.0";
        src = code-adapter-src;
        cargoDeps = cowboy-cargo-deps;
        cargoBuildFlags = [
          "--no-default-features"
          "--features"
          "code-adapter"
          "--bin"
          "cowboy-code-adapter"
        ];
        doCheck = false;
        meta = {
          description = "Filesystem and Git adapter for Cowboy Machine";
          mainProgram = "cowboy-code-adapter";
        };
      };

      cowboy-zed-adapter = pkgs.rustPlatform.buildRustPackage {
        pname = "cowboy-zed-adapter";
        version = "0.1.0";
        src = zed-adapter-src;
        cargoLock = {
          lockFile = ./zed-adapter/Cargo.lock;
          outputHashes = {
            "proto-0.1.0" =
              "sha256-sAjiYGwmQB+Zzb/b7PGm4Nfv36Vb0myqKIBfpuHGTik=";
          };
        };
        nativeBuildInputs = [ pkgs.protobuf ];
        meta = {
          description = "GPL-isolated Zed protocol adapter for Cowboy Code";
          license = pkgs.lib.licenses.gpl3Plus;
          mainProgram = "cowboy-zed-adapter";
        };
      };

      # Zed's official remote-development flow installs a release server on
      # the target host rather than compiling the editor workspace there.
      # Pin the exact preview release that corresponds to ZED_REVISION in the
      # adapter. This keeps Cowboy's instance reproducible and independent of
      # the user's ~/.zed_server lifecycle.
      cowboy-zed-server = pkgs.runCommand "cowboy-zed-server-1.13.0" {
        src = pkgs.fetchurl {
          url =
            "https://github.com/zed-industries/zed/releases/download/v1.13.0-pre/zed-remote-server-linux-x86_64.gz";
          hash = "sha256-+E10MkfNuSORMNvhyRm3Ij5UfM5mrWwKSVkj+FJGQ+Y=";
        };
        nativeBuildInputs = [ pkgs.gzip ];
        meta = {
          description = "Pinned isolated Zed remote server for Cowboy Code";
          license = pkgs.lib.licenses.gpl3Plus;
          mainProgram = "cowboy-zed-server";
        };
      } ''
          mkdir -p "$out/bin"
          gzip -dc "$src" > "$out/bin/cowboy-zed-server"
          chmod 0555 "$out/bin/cowboy-zed-server"
      '';

      cowboy-source-boundary = pkgs.runCommand "cowboy-source-boundary" { } ''
        test ! -e ${cowboy-src}/docs
        test ! -e ${cowboy-src}/web/public
        test -e ${cowboy-src}/web/src/protocol.ts
        test ! -e ${cowboy}/bin/cowboy-machine
        test ! -e ${cowboy}/bin/cowboy-machine-install
        test -x ${cowboy-machine}/bin/cowboy-machine
        test -x ${cowboy-machine}/bin/cowboy-machine-install
        test -x ${cowboy-code-adapter}/bin/cowboy-code-adapter
        touch "$out"
      '';

      cowboy-zed-integration = pkgs.runCommand "cowboy-zed-integration" {
        nativeBuildInputs = [ pkgs.coreutils pkgs.jq pkgs.netcat-openbsd ];
      } ''
        runtime="$TMPDIR/cowboy-zed"
        export HOME="$runtime/home"
        export XDG_CACHE_HOME="$runtime/cache"
        export XDG_CONFIG_HOME="$runtime/config"
        export XDG_DATA_HOME="$runtime/data"
        export XDG_STATE_HOME="$runtime/state"
        mkdir -p "$HOME" "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME" \
          "$XDG_DATA_HOME" "$XDG_STATE_HOME"
        ${cowboy-zed-adapter}/bin/cowboy-zed-adapter serve \
          --socket "$runtime/adapter.sock" \
          --zed-server ${cowboy-zed-server}/bin/cowboy-zed-server \
          --state-dir "$runtime/state" &
        adapter_pid=$!
        trap 'kill "$adapter_pid" 2>/dev/null || true; wait "$adapter_pid" 2>/dev/null || true' EXIT

        ${cowboy-zed-adapter}/bin/cowboy-zed-adapter probe \
          --socket "$runtime/adapter.sock" --wait-ms 30000 >/dev/null
        printf '%s\n' \
          '{"type":"openWorktree","path":"${./.}","trusted":true}' \
          | nc -N -U "$runtime/adapter.sock" \
          | jq -e '.type == "worktree" and .state == "ready" and .leases == 1' \
          >/dev/null
        printf '%s\n' \
          '{"type":"openBuffer","worktree":"${./.}","path":"Cargo.toml","leaseId":"nix-integration"}' \
          | nc -N -U "$runtime/adapter.sock" \
          | jq -e '.type == "buffer" and .path == "Cargo.toml" and .leases == 1' \
          >/dev/null
        printf '%s\n' \
          '{"type":"closeBuffer","worktree":"${./.}","path":"Cargo.toml","leaseId":"nix-integration"}' \
          | nc -N -U "$runtime/adapter.sock" \
          | jq -e '.type == "buffer" and .path == "Cargo.toml" and .leases == 0' \
          >/dev/null
        printf '%s\n' \
          '{"type":"closeWorktree","path":"${./.}"}' \
          | nc -N -U "$runtime/adapter.sock" \
          | jq -e '.type == "worktree" and .leases == 0' >/dev/null
        touch "$out"
      '';
    in
    {
      packages.${system} = {
        default = cowboy;
        cowboy = cowboy;
        cowboy-machine = cowboy-machine;
        cowboy-code-adapter = cowboy-code-adapter;
        cowboy-zed-adapter = cowboy-zed-adapter;
        cowboy-zed-server = cowboy-zed-server;
        cowboy-web = cowboy-web;
      };

      # `cowboy`'s buildRustPackage check phase runs the Rust tests; cowboy-web's
      # build runs TypeScript checking before Vite. Developer lint/test policy is
      # additionally enforced by `just check` in CI.
      checks.${system} = {
        inherit cowboy cowboy-machine cowboy-code-adapter cowboy-source-boundary cowboy-web
          cowboy-zed-integration
          cowboy-zed-adapter cowboy-zed-server;
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
