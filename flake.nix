{
  description = "cowboy — drive coding-agent CLIs from anywhere over ACP, with one shared live progress";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  # Exact pinned Rust toolchain from rust-toolchain.toml. rustc 1.97.1 is
  # measurably faster than the nixpkgs 1.95 default (clean check 19.8s -> 13.3s,
  # clean debug build 30.3s -> 19.5s) while staying within the same compiler
  # family; the overlay also makes the dev shell and every Nix build use the
  # exact same rustc/cargo pair.
  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };
      rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      rustPlatform = pkgs.makeRustPlatform {
        cargo = rustToolchain;
        rustc = rustToolchain;
      };
      deno = import ./nix/deno.nix { inherit pkgs; };
      buildDenoViteApp = import ./nix/deno-vite-app.nix {
        inherit pkgs deno;
        lib = pkgs.lib;
      };

      # Every Rust release source carries the generic Plugin SDK plus the Agent
      # Provider capability SDK. The Controller and Machine also compile the
      # first-party manifests into their typed fallback Catalog, so keep those exact inputs
      # in the narrow Rust closure without pulling release tooling or npm locks
      # into component identities.
      provider-sdk-files = [
        ./components/provider-sdk/Cargo.toml
        ./components/provider-sdk/src
      ];
      plugin-sdk-files = [
        ./components/plugin-sdk/Cargo.toml
        ./components/plugin-sdk/src
      ];
      provider-manifest-files = [
        ./plugins/claude-code/provider.json
        ./plugins/claude-code/plugin.json
        ./plugins/claude-deepseek/provider.json
        ./plugins/claude-deepseek/plugin.json
        ./plugins/codex/provider.json
        ./plugins/codex/plugin.json
        ./plugins/codex-deepseek/provider.json
        ./plugins/codex-deepseek/plugin.json
        ./plugins/gemini/provider.json
        ./plugins/gemini/plugin.json
        ./plugins/grok/provider.json
        ./plugins/grok/plugin.json
        ./plugins/zed/plugin.json
        ./plugins/zed/contract.json
        ./components/registry.json
        ./components/plugin-contract/schema.json
        ./components/code-intelligence/contract.json
      ];
      plugin-contract-files = plugin-sdk-files ++ provider-sdk-files ++ provider-manifest-files;

      # Backend and frontend are independent deployment artifacts. Keep this
      # closure explicit: docs, Web, native-shell, and operational edits must
      # not change the Rust package's store path and restart the API unit.
      # protocol.ts is the sole frontend input because Rust contract tests
      # deliberately compile-check its wire tags.
      cowboy-src = pkgs.lib.fileset.toSource {
        root = ./.;
        fileset = pkgs.lib.fileset.unions ([
          ./Cargo.toml
          ./Cargo.lock
          ./src
          ./migrations
          ./web/src/protocol.ts
        ] ++ plugin-contract-files);
      };

      # Machine has a deliberately tiny source closure and is packaged
      # separately. Ordinary API, SPA, ACP, or worker edits therefore leave
      # its ExecStart path unchanged.
      machine-src = pkgs.lib.fileset.toSource {
        root = ./.;
        fileset = pkgs.lib.fileset.unions ([
          ./Cargo.toml
          ./Cargo.lock
          ./src/lib.rs
          ./src/main.rs
          ./src/cli.rs
          ./src/claude_shell.rs
          ./src/grok.rs
          ./src/legacy_provider_release.rs
          ./src/machine_broker.rs
          ./src/machine_cli.rs
          ./src/machine_auth.rs
          ./src/machine_components.rs
          ./src/machine_install.rs
          ./src/machine_protocol.rs
          ./src/machine_plugins.rs
          ./src/provider/deepseek_cache.rs
          ./src/provider/deepseek_context.rs
          ./src/provider_behavior.rs
          ./src/provider_usage_spool.rs
          ./src/provider_catalog.rs
          ./src/runtime_wire.rs
          ./src/service_identity.rs
          ./src/session_workspace.rs
          ./src/workspace_roots.rs
          ./src/bin/cowboy-machine-install.rs
          ./src/bin/cowboy-machine.rs
        ] ++ plugin-contract-files);
      };

      code-adapter-src = pkgs.lib.fileset.toSource {
        root = ./.;
        fileset = pkgs.lib.fileset.unions ([
          ./Cargo.toml
          ./Cargo.lock
          ./src/lib.rs
          ./src/code_adapter.rs
          ./src/code_review.rs
          ./src/files.rs
          ./src/workspace_roots.rs
          ./src/bin/cowboy-code-adapter.rs
        ] ++ plugin-sdk-files ++ provider-sdk-files);
      };

      zed-adapter-src = pkgs.lib.cleanSource ./plugins/zed/adapter;

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
        ./src/bin/cowboy-codex-app-server.rs
        ./src/cgroup.rs
        ./src/claude_shell.rs
        ./src/grok.rs
        ./src/provider/deepseek_cache.rs
        ./src/provider/deepseek_context.rs
        ./src/provider/mod.rs
        ./src/provider_catalog.rs
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

      # The SPA uses Cowboy's local two-layer builder: a deps-only FOD
      # (vendored npm cache, keyed by the lockfiles → depsHash below) + a normal
      # content-addressed offline build. Any source edit rebuilds automatically;
      # only refresh depsHash when web/deno.lock or web/package.json change
      # (lib.fakeHash → build → copy "got"). Local component packages are
      # copied only to resolve their file: manifests; their source stays in the
      # ordinary content-addressed build rather than the dependency cache.
      cowboy-web = buildDenoViteApp {
        pname = "cowboy";
        version = "0.1.0";
        src = pkgs.lib.cleanSource ./.;
        localPackages = [
          "components/app-shell"
          "components/provider-ui"
          "components/state-store"
          "components/state-sync"
          "components/state-sync-idb"
        ];
        depsHash = "sha256-kR5fiqN0MjVyJZSHxdfFsCjtKys+1f5oQXoD/HgPiFI=";
      };

      # This host's pinned Nixpkgs still has the first fetchCargoVendor
      # implementation, which downloads through crates.io's rate-limited API.
      # crates.io now rejects that bulk endpoint with a data-access 403. Newer
      # Nixpkgs uses the official immutable static CDN for exactly this reason.
      # Patch only the vendoring helper inside the FOD; Cargo.lock checksums and
      # the aggregate cargo hash remain fully enforced.
      cowboy-cargo-deps = rustPlatform.fetchCargoVendor {
        pname = "cowboy";
        version = "0.1.0";
        src = cowboy-src;
        hash = "sha256-ct8KpXOAuIJZ4vQpMmwMuTeOzhdNLVmNWjNGa+HryJo=";
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
      cowboy = rustPlatform.buildRustPackage {
        pname = "cowboy";
        version = "0.1.0";
        src = cowboy-src;
        cargoDeps = cowboy-cargo-deps;
        # jemalloc's configure probes use -Werror while Nix enables
        # _FORTIFY_SOURCE; keep C probes optimized so glibc does not reject
        # that valid combination during debug test builds.
        CFLAGS = "-O1";
        # Cowboy is a low-throughput control plane with bursty JSON restore
        # allocations. Prefer promptly releasing memory over allocator
        # throughput: one arena, no thread cache, no dirty/muzzy decay, and no
        # retained virtual mappings. abort_conf makes a typo fail the build/run
        # instead of silently falling back to a higher-residency default.
        JEMALLOC_SYS_WITH_MALLOC_CONF =
          "abort_conf:true,background_thread:true,narenas:1,tcache:false,dirty_decay_ms:0,muzzy_decay_ms:0,retain:false,metadata_thp:disabled,thp:never";
        cargoBuildFlags = [
          "--bin"
          "cowboy"
          "--bin"
          "cowboy-acp-worker"
          "--bin"
          "cowboy-codex-app-server"
        ];
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = [ pkgs.openssl ];
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

      cowboy-machine = rustPlatform.buildRustPackage {
        pname = "cowboy-machine";
        version = "0.1.0";
        src = machine-src;
        cargoDeps = cowboy-cargo-deps;
        cargoBuildFlags = [
          "--no-default-features"
          "--features"
          "machine-host"
          "--bin"
          "cowboy"
          "--bin"
          "cowboy-machine"
          "--bin"
          "cowboy-machine-install"
        ];
        nativeBuildInputs = [ pkgs.makeWrapper pkgs.pkg-config ];
        buildInputs = [ pkgs.openssl ];
        postInstall = ''
          wrapProgram $out/bin/cowboy \
            --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.openssh ]}
          wrapProgram $out/bin/cowboy-machine \
            --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.openssh ]}
        '';
        doCheck = false;
        meta = {
          description = "Stable Cowboy Machine host for detached ACP workers";
          mainProgram = "cowboy-machine";
        };
      };

      cowboy-code-adapter = rustPlatform.buildRustPackage {
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
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = [ pkgs.openssl ];
        doCheck = false;
        meta = {
          description = "Filesystem and Git adapter for Cowboy Machine";
          mainProgram = "cowboy-code-adapter";
        };
      };

      cowboy-zed-adapter = rustPlatform.buildRustPackage {
        pname = "cowboy-zed-adapter";
        version = "1.1.2";
        src = zed-adapter-src;
        cargoLock = {
          lockFile = ./plugins/zed/adapter/Cargo.lock;
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

      release-revision = if self ? rev then self.rev else null;
      release-source = lane: bootstrap: {
        schema = 1;
        component = "cowboy";
        inherit lane;
        repository = "git@github.com:dravengarden/cowboy.git";
        revision = release-revision;
        dirty = release-revision == null;
      } // pkgs.lib.optionalAttrs bootstrap { bootstrap = true; };

      # Fully managed NixOS hosts consume stable component profiles instead of
      # embedding these store paths in every system generation. Web is its own
      # zero-restart lane; controller and resident Machine maintenance remain
      # explicit, independently recoverable transactions.
      cowboy-controller-release =
        pkgs.runCommand "cowboy-controller-release" { } ''
          mkdir -p "$out/bin" "$out/etc/cowboy-release"
          ln -s ${cowboy}/bin/cowboy "$out/bin/cowboy"
          cat >"$out/etc/cowboy-release/source.json" <<'EOF'
          ${builtins.toJSON (release-source "controller" false)}
          EOF
        '';

      cowboy-web-release = pkgs.runCommand "cowboy-web-release" { } ''
        mkdir -p "$out/share/cowboy" "$out/etc/cowboy-release"
        ln -s ${cowboy-web} "$out/share/cowboy/web"
        cat >"$out/etc/cowboy-release/source.json" <<'EOF'
        ${builtins.toJSON (release-source "web" false)}
        EOF
      '';

      machine-release = bootstrap:
        pkgs.runCommand
          (if bootstrap then "cowboy-machine-bootstrap-release" else "cowboy-machine-release")
          { nativeBuildInputs = [ pkgs.makeWrapper ]; } ''
        mkdir -p "$out/bin" "$out/libexec" "$out/etc/cowboy-release"
        ln -s ${cowboy-machine}/bin/cowboy-machine \
          "$out/libexec/cowboy-machine"
        ${
          if bootstrap then
            ''ln -s "$out/libexec/cowboy-machine" "$out/bin/cowboy-machine"''
          else
            ''makeWrapper "$out/libexec/cowboy-machine" "$out/bin/cowboy-machine" \
              --add-flags "--desired-generation ${worker-generation}"''
        }
        ln -s ${cowboy-machine}/bin/cowboy-machine-install \
          "$out/bin/cowboy-machine-install"
        ln -s ${cowboy-machine}/bin/cowboy \
          "$out/bin/cowboy"
        ln -s ${cowboy}/bin/cowboy-acp-worker "$out/bin/cowboy-acp-worker"
        ln -s ${cowboy}/bin/cowboy-codex-app-server \
          "$out/bin/cowboy-codex-app-server"
        ln -s ${cowboy-code-adapter}/bin/cowboy-code-adapter \
          "$out/bin/cowboy-code-adapter"
        ln -s ${cowboy-zed-adapter}/bin/cowboy-zed-adapter \
          "$out/bin/cowboy-zed-adapter"
        ln -s ${cowboy-zed-server}/bin/cowboy-zed-server \
          "$out/bin/cowboy-zed-server"
        machine_help="$(${cowboy-machine}/bin/cowboy-machine --help)"
        printf '%s\n' "$machine_help" \
          | ${pkgs.gnugrep}/bin/grep -F -- '--socket' >/dev/null
        if printf '%s\n' "$machine_help" \
          | ${pkgs.gnugrep}/bin/grep -F -- '--compat-socket' >/dev/null; then
          echo "cowboy-machine still exposes retired --compat-socket" >&2
          exit 1
        fi
        cat >"$out/etc/cowboy-release/source.json" <<'EOF'
        ${builtins.toJSON ((release-source "machine" bootstrap) // {
          workerGeneration = cowboy.workerGeneration;
        })}
        EOF
      '';
      cowboy-machine-bootstrap-release = machine-release true;
      cowboy-machine-release = machine-release false;

      cowboy-source-boundary = pkgs.runCommand "cowboy-source-boundary" { } ''
        test ! -e ${cowboy-src}/docs
        test ! -e ${cowboy-src}/web/public
        test -e ${cowboy-src}/web/src/protocol.ts
        test -e ${cowboy-src}/components/provider-sdk/Cargo.toml
        test -e ${cowboy-src}/components/plugin-sdk/Cargo.toml
        test -e ${cowboy-src}/plugins/codex/provider.json
        test ! -e ${cowboy-src}/components/provider-runtime/lock.json
        test -e ${machine-src}/components/provider-sdk/Cargo.toml
        test -e ${machine-src}/components/plugin-sdk/Cargo.toml
        test -e ${machine-src}/plugins/gemini/provider.json
        test ! -e ${machine-src}/components/provider-runtime/lock.json
        test -e ${code-adapter-src}/components/provider-sdk/Cargo.toml
        test -e ${code-adapter-src}/components/plugin-sdk/Cargo.toml
        test ! -e ${code-adapter-src}/providers
        test -e ${machine-src}/src/provider/deepseek_cache.rs
        test -e ${machine-src}/src/grok.rs
        test -e ${machine-src}/src/provider/deepseek_context.rs
        test -e ${machine-src}/src/machine_plugins.rs
        test -e ${machine-src}/src/provider_behavior.rs
        test -e ${machine-src}/src/provider_catalog.rs
        test -e ${machine-src}/src/provider_usage_spool.rs
        test -e ${machine-src}/src/session_workspace.rs
        test ! -e ${cowboy}/bin/cowboy-machine
        test ! -e ${cowboy}/bin/cowboy-machine-install
        test -x ${cowboy}/bin/cowboy-codex-app-server
        test -x ${cowboy-machine}/bin/cowboy-machine
        test -x ${cowboy-machine}/bin/cowboy-machine-install
        test -x ${cowboy-machine}/bin/cowboy
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
        cowboy-controller-release = cowboy-controller-release;
        cowboy-web-release = cowboy-web-release;
        cowboy-machine-bootstrap-release = cowboy-machine-bootstrap-release;
        cowboy-machine-release = cowboy-machine-release;
      };

      # `cowboy`'s buildRustPackage check phase runs the Rust tests; cowboy-web's
      # build runs TypeScript checking before Vite. Developer lint/test policy is
      # additionally enforced by `just check` in CI.
      checks.${system} = {
        inherit cowboy cowboy-machine cowboy-code-adapter cowboy-source-boundary
          cowboy-web cowboy-controller-release cowboy-web-release
          cowboy-machine-bootstrap-release cowboy-machine-release
          cowboy-zed-integration cowboy-zed-adapter cowboy-zed-server;
      };

      devShells.${system}.default = pkgs.mkShell {
        # See the controller package note above. This only affects C build
        # scripts; rustc keeps the profile selected by Cargo.
        CFLAGS = "-O1";
        # Rust toolchain plus opt-in sccache, and the frontend toolchain
        # (Cowboy's pinned Deno + node 24 for any node-shaped tool that
        # deno's npm interop can't shim).
        COWBOY_DENO_VERSION = deno.version;
        nativeBuildInputs = with pkgs; [
          rustToolchain
          sccache
          cargo-nextest
          cargo-deny
          cargo-machete
          brotli
          curl
          git
          gnutar
          gzip
          just
          jq
          go
          nodejs_24
        ] ++ [ deno ];

        shellHook = ''
          echo "cowboy dev shell — rust + optional sccache + deno"
          sccache --version >/dev/null 2>&1 && echo "sccache: $(sccache --version)"
          deno --version 2>/dev/null | head -1
        '';
      };
    };
}
