{
  description = "cowboy — drive coding-agent CLIs from anywhere over ACP, with one shared live progress";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  # Keep the dev shell and every release build on the exact rustc/cargo pair
  # declared in rust-toolchain.toml. SDK minimum compiler versions are separate
  # compatibility floors and need not rise with the project's build toolchain.
  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  # Machine host fixes retain the separately accepted detached-worker bundle.
  # Advance this exact source only with worker/adapter maintenance acceptance.
  inputs.cowboy-workers.url = "github:dravengarden/cowboy/406471a28de430debf6f8363b44abc3e621589d7";

  outputs = { self, nixpkgs, rust-overlay, cowboy-workers }:
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
      cowboy-nodejs = import ./nix/nodejs.nix { inherit pkgs; };
      execution-runtime = import ./nix/execution-runtime.nix { inherit pkgs; };
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
      # First-party plugin manifests are discovered by name so adding a
      # Provider does not edit this closure list.
      provider-manifest-files = [
        (pkgs.lib.fileset.fileFilter (file:
          file.name == "plugin.json" || file.name == "provider.json" ||
          file.name == "contract.json"
        ) ./plugins)
        ./components/registry.json
        ./components/plugin-contract/schema.json
        ./components/code-intelligence/contract.json
      ];
      plugin-contract-files = plugin-sdk-files ++ provider-sdk-files ++ provider-manifest-files;
      # Bootstrap hosts and signed authentication fixtures are discovered by
      # build.rs and the Controller tests. Keep package payloads in the filtered
      # source so the hermetic check phase exercises the same Catalog cutover.
      plugin-host-files = [
        (pkgs.lib.fileset.fileFilter (file: file.hasExt "json") ./examples/telemetry)
        (pkgs.lib.fileset.fileFilter (file:
          file.name == "host.json" || file.name == "plugin.json" ||
          file.name == "authentication.json"
        ) ./examples/authentication)
        (pkgs.lib.fileset.fileFilter (file:
          file.name == "host.json" || file.hasExt "js" || file.hasExt "css"
        ) ./plugins)
      ];

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
          ./build.rs
          ./src
          ./tools/worker-registry-input.ts
          ./components/worker-registry-input.json
          ./migrations
          ./web/src/protocol.ts
          ./contracts/code-buffer-client.fixture.json
          ./contracts/code-buffer-sync.fixture.json
          ./contracts/code-buffer-sync-budget.fixture.json
          ./contracts/code-buffer-navigation.fixture.json
          ./contracts/code-buffer-destination.fixture.json
          ./plugins/zed/adapter/fixtures/content.json
          ./plugins/zed/adapter/fixtures/text.json
          ./tests/fixtures/otel-client.json
          ./tests/fixtures/composition-v1.json
          ./tests/fixtures/telemetry-resolution-surface.json
          ./tests/fixtures/telemetry-recovery-surface.json
          ./tests/fixtures/telemetry-binding-surface.json
          ./tests/fixtures/plugin-lifecycle-history.json
        ] ++ plugin-contract-files ++ plugin-host-files);
      };

      # Machine has a deliberately tiny source closure and is packaged
      # separately. Ordinary API, SPA, ACP, or worker edits therefore leave
      # its ExecStart path unchanged.
      machine-src = pkgs.lib.fileset.toSource {
        root = ./.;
        fileset = pkgs.lib.fileset.unions ([
          ./Cargo.toml
          ./Cargo.lock
          ./build.rs
          ./src/lib.rs
          ./src/main.rs
          ./src/cli.rs
          ./src/composition
          ./tests/fixtures/composition-v1.json
          ./src/claude_shell.rs
          ./src/cgroup.rs
          ./src/execution_environment.rs
          ./src/execution_protocol.rs
          ./src/execution_host.rs
          ./src/execution_host
          ./src/bin/cowboy-execution-host.rs
          ./src/code_buffer_read.rs
          ./src/code_buffer_read
          ./plugins/zed/adapter/fixtures/content.json
          ./plugins/zed/adapter/fixtures/text.json
          ./src/first_party_sources.rs
          ./src/plugin_auth_probe.rs
          ./src/plugin_host.rs
          ./src/plugin_host_bundle.rs
          ./src/plugin_process.rs
          ./src/plugin_process
          ./src/plugin_runtime_args.rs
          ./src/legacy_provider_release.rs
          ./src/machine_broker.rs
          ./src/machine_broker
          ./src/machine_code_plugins.rs
          ./src/machine_code_plugins
          ./src/machine_cli.rs
          ./src/machine_cli
          ./src/machine_auth.rs
          ./src/machine_components.rs
          ./src/machine_components
          ./src/machine_install.rs
          ./src/machine_install
          ./src/session_deletion_admission.rs
          ./src/machine_protocol.rs
          ./src/machine_protocol
          ./src/machine_plugins.rs
          ./src/machine_plugins
          ./src/operation_budget.rs
          ./src/owned_json.rs
          ./src/telemetry_plugin.rs
          ./src/telemetry_plugin/writer_admission.rs
          ./src/telemetry_plugin/writer_admission
          ./src/otlp.rs
          ./src/provider/deepseek_cache.rs
          ./src/provider/deepseek_context.rs
          ./src/provider_behavior.rs
          ./src/provider_usage_spool.rs
          ./src/provider_catalog.rs
          ./src/runtime_wire.rs
          ./src/runtime_trace.rs
          ./src/service_identity.rs
          ./src/session_workspace.rs
          ./src/workspace_extensions.rs
          ./src/workspace_extensions
          ./src/workspace_roots.rs
          ./src/bin/cowboy-machine-install.rs
          ./src/bin/cowboy-machine.rs
        ] ++ plugin-contract-files ++ [
          (pkgs.lib.fileset.fileFilter (file: file.name == "host.json") ./plugins)
        ]);
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


      # Only behavior that runs inside a detached session contributes to the
      # pool generation. A control-plane-only change updates Cowboy without
      # draining live ACP sessions.
      worker-generation-files = pkgs.lib.fileset.toList (pkgs.lib.fileset.unions ([
        ./Cargo.toml
        ./Cargo.lock
        ./build.rs
        ./worker-generation.txt
        ./src/acp.rs
        ./src/agent_model.rs
        ./src/agent_sink.rs
        ./src/bin/cowboy-codex-app-server.rs
        ./src/cgroup.rs
        ./src/claude_shell.rs
        ./src/first_party_sources.rs
        ./src/plugin_runtime_args.rs
        ./src/provider/deepseek_cache.rs
        ./src/provider/deepseek_context.rs
        ./src/provider/managed_config.rs
        ./src/provider/mod.rs
        ./src/provider_behavior.rs
        ./src/provider_catalog.rs
        ./src/runtime_wire.rs
        ./src/execution_environment.rs
        ./src/execution_protocol.rs
        ./src/runtime_trace.rs
        ./src/worker.rs
        ./src/worker_execution.rs
        ./src/worker_telemetry.rs
        ./src/bin/cowboy-acp-worker.rs
      ] ++ plugin-contract-files ++ [
        (pkgs.lib.fileset.fileFilter (file: file.name == "host.json") ./plugins)
      ]));
      worker-registry-input = builtins.fromJSON (builtins.readFile ./components/worker-registry-input.json);
      worker-registry-digest = assert worker-registry-input.schema == 1;
        assert builtins.match "[0-9a-f]{64}" worker-registry-input.registry_sha256 != null;
        worker-registry-input.registry_sha256;
      worker-generation = "worker-" + builtins.substring 0 20 (
        builtins.hashString "sha256" (
          pkgs.lib.concatMapStringsSep ":"
            (path: if path == ./components/registry.json then worker-registry-digest
              else builtins.hashFile "sha256" path)
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
        nodejs = cowboy-nodejs;
        src = pkgs.lib.cleanSource ./.;
        localPackages = [
          "components/app-shell"
          "components/provider-authoring"
          "components/provider-ui"
          "components/state-store"
          "components/state-sync"
          "components/state-sync-idb"
        ];
        depsHash = "sha256-CfVJESKmXrQECBmPrP3wK8NM3tqfFw6Z5C+GtwQof1A=";
      };

      # This host's pinned Nixpkgs still has the first fetchCargoVendor
      # implementation, which downloads through crates.io's rate-limited API.
      # crates.io now rejects that bulk endpoint with a data-access 403. Newer
      # Nixpkgs uses the official immutable static CDN for exactly this reason.
      # Patch only the vendoring helper inside the FOD; Cargo.lock checksums and
      # the aggregate cargo hash remain fully enforced.
      staticCratesVendorPatch = ''
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

      cowboy-cargo-deps = rustPlatform.fetchCargoVendor {
        pname = "cowboy";
        version = "0.1.0";
        src = cowboy-src;
        hash = "sha256-9WRr2ZeOi1LtY85gSMqSoxsmf1qF6cxFRtZnfj6Mdl0=";
        preBuild = staticCratesVendorPatch;
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
        # This derived digest must never become a stale worker-generation pin.
        # Enforce it inside the immutable worker build as well as the root gate.
        preBuild = ''
          ${deno}/bin/deno run --allow-read tools/worker-registry-input.ts
        '';
        buildInputs = [ pkgs.openssl ];
        nativeCheckInputs = [ pkgs.cacert pkgs.gitMinimal pkgs.openssh deno ];
        preCheck = ''
          export SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt
        '';
        # current_exe resolves the release symlink to this package, so the
        # closed Plugin runtime must be beside the real executable too.
        postInstall = ''
          ln -s ${deno}/bin/deno "$out/bin/cowboy-plugin-js"
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
          "--bin"
          "cowboy-execution-host"
        ];
        nativeBuildInputs = [ pkgs.makeWrapper pkgs.pkg-config ];
        buildInputs = [ pkgs.openssl ];
        postInstall = ''
          ln -s ${deno}/bin/deno "$out/bin/cowboy-plugin-js"
          # User services have a closed PATH. Supply the connection tool while
          # retaining the OS user's existing gh login and configuration.
          wrapProgram $out/bin/cowboy \
            --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.openssh pkgs.gh deno ]}
          wrapProgram $out/bin/cowboy-machine \
            --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.openssh pkgs.gh deno ]}
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

      # Independent immutable release tooling for external Plugin publishers
      # such as Cardea. Its SDK validator and Ed25519 tool are closure-pinned.
      cowboy-plugin-pack = rustPlatform.buildRustPackage {
        pname = "cowboy-plugin-pack";
        version = (builtins.fromTOML
          (builtins.readFile ./components/plugin-sdk/Cargo.toml)).package.version;
        src = cowboy-src;
        cargoDeps = cowboy-cargo-deps;
        cargoBuildFlags = [ "-p" "cowboy-plugin-sdk" "--bin" "cowboy-plugin-pack" ];
        cargoTestFlags = [ "-p" "cowboy-plugin-sdk" ];
        nativeBuildInputs = [ pkgs.makeWrapper pkgs.openssh ];
        postFixup = ''
          wrapProgram "$out/bin/cowboy-plugin-pack" --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.openssh ]}
        '';
        meta.mainProgram = "cowboy-plugin-pack";
      };

      zedRuntime = import ./plugins/zed/runtime/default.nix {
        inherit pkgs rustToolchain rustPlatform;
      };
      inherit (zedRuntime) cowboy-zed-adapter cowboy-zed-adapter-portable cowboy-zed-server;

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
          ln -s ${deno}/bin/deno "$out/bin/cowboy-plugin-js"
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

      execution-configuration = pkgs.writeText "cowboy-execution.json" (builtins.toJSON {
        schema = 1;
        host_command = "${cowboy-machine}/bin/cowboy-execution-host";
        executor = {
          command = "${execution-runtime}/bin/codex";
          sha256 = execution-runtime.executorDigest;
          version = execution-runtime.executorVersion;
        };
        retention = {
          command = "${pkgs.nix}/bin/nix-store";
          closure = "${execution-runtime}";
        };
      });

      retained-worker-bundle = cowboy-workers.packages.${system}.cowboy-machine-release;
      retained-worker-package = cowboy-workers.packages.${system}.cowboy;
      retained-worker-interface-files = pkgs.lib.fileset.toList (pkgs.lib.fileset.unions ([
        ./Cargo.toml
        ./Cargo.lock
        ./src/runtime_wire.rs
        ./src/execution_protocol.rs
        ./src/execution_environment.rs
        ./src/machine_protocol.rs
        ./src/machine_protocol
      ] ++ plugin-sdk-files ++ provider-sdk-files));
      retained-worker-interface-compatible = pkgs.lib.all (path:
        let
          relative = pkgs.lib.removePrefix "${toString ./.}/" (toString path);
          prior = "${cowboy-workers.outPath}/${relative}";
        in builtins.pathExists prior && builtins.hashFile "sha256" path == builtins.hashFile "sha256" prior
      ) retained-worker-interface-files;
      machine-release = bootstrap: retain-workers:
        pkgs.runCommand
          (if bootstrap then "cowboy-machine-bootstrap-release" else "cowboy-machine-release")
          { nativeBuildInputs = [ pkgs.makeWrapper ]; } ''
        mkdir -p "$out/bin" "$out/libexec" "$out/etc/cowboy-release"
        ln -s ${cowboy-machine}/bin/cowboy-machine \
          "$out/libexec/cowboy-machine"
        ${
          if bootstrap then
            ''makeWrapper "$out/libexec/cowboy-machine" "$out/bin/cowboy-machine" \
              --set COWBOY_DEFAULT_EXECUTION_CONFIG ${execution-configuration}''
          else
            ''makeWrapper "$out/libexec/cowboy-machine" "$out/bin/cowboy-machine" \
              --set COWBOY_DEFAULT_EXECUTION_CONFIG ${execution-configuration} \
              --add-flags "--desired-generation ${if retain-workers then retained-worker-package.workerGeneration else worker-generation}"''
        }
        # Registration finds companions beside the native current_exe. Keep
        # both the native executable and its wrapper in this complete bundle.
        cp ${cowboy-machine}/bin/cowboy-machine-install \
          "$out/bin/cowboy-machine-install"
        cp ${cowboy-machine}/bin/cowboy-execution-host \
          "$out/bin/cowboy-execution-host"
        cp ${cowboy-machine}/bin/.cowboy-wrapped "$out/bin/.cowboy-wrapped"
        makeWrapper "$out/bin/.cowboy-wrapped" "$out/bin/cowboy" \
          --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.openssh pkgs.gh deno ]}
        ${if retain-workers then ''
          for command in cowboy-plugin-js cowboy-acp-worker cowboy-codex-app-server cowboy-code-adapter cowboy-zed-adapter cowboy-zed-server; do
            ln -s ${retained-worker-bundle}/bin/"$command" "$out/bin/$command"
          done
          cp ${retained-worker-bundle}/etc/cowboy-release/source.json \
            "$out/etc/cowboy-release/retained-worker-source.json"
        '' else ''
          ln -s ${deno}/bin/deno "$out/bin/cowboy-plugin-js"
          ln -s ${cowboy}/bin/cowboy-acp-worker "$out/bin/cowboy-acp-worker"
          ln -s ${cowboy}/bin/cowboy-codex-app-server \
            "$out/bin/cowboy-codex-app-server"
          ln -s ${cowboy-code-adapter}/bin/cowboy-code-adapter \
            "$out/bin/cowboy-code-adapter"
          ln -s ${cowboy-zed-adapter}/bin/cowboy-zed-adapter \
            "$out/bin/cowboy-zed-adapter"
          ln -s ${cowboy-zed-server}/bin/cowboy-zed-server \
            "$out/bin/cowboy-zed-server"
        ''}
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
          workerGeneration = if retain-workers then retained-worker-package.workerGeneration else cowboy.workerGeneration;
          sessionDeletionJournal = {
            readerSchema = 1;
            writerSchema = 0;
          };
        })}
        EOF
      '';
      cowboy-machine-bootstrap-release = machine-release true false;
      cowboy-machine-release = machine-release false false;
      cowboy-machine-host-release = assert retained-worker-interface-compatible;
        machine-release false true;

      cowboy-source-boundary = pkgs.runCommand "cowboy-source-boundary" { } ''
        test ! -e ${cowboy-src}/docs
        test ! -e ${cowboy-src}/web/public
        test -e ${cowboy-src}/web/src/protocol.ts
        test -e ${cowboy-src}/contracts/code-buffer-client.fixture.json
        test -e ${cowboy-src}/contracts/code-buffer-sync.fixture.json
        test -e ${cowboy-src}/contracts/code-buffer-sync-budget.fixture.json
        test -e ${cowboy-src}/contracts/code-buffer-navigation.fixture.json
        test -e ${cowboy-src}/contracts/code-buffer-destination.fixture.json
        test -e ${cowboy-src}/components/provider-sdk/Cargo.toml
        test -e ${cowboy-src}/components/plugin-sdk/Cargo.toml
        test -e ${cowboy-src}/plugins/codex/provider.json
        test -e ${cowboy-src}/plugins/zed/plugin.json
        test -e ${cowboy-src}/plugins/zed/adapter/fixtures/content.json
        test -e ${cowboy-src}/plugins/zed/adapter/fixtures/text.json
        test ! -e ${cowboy-src}/plugins/zed/adapter/src/main.rs
        test -e ${cowboy-src}/build.rs
        test -e ${cowboy-src}/plugins/grok/host.json
        test -e ${cowboy-src}/plugins/grok/collector/index.js
        test ! -e ${cowboy-src}/plugins/grok/ui/index.js
        test -e ${cowboy-src}/examples/authentication/password/host.json
        test -e ${cowboy-src}/examples/authentication/password/plugin.json
        test -e ${cowboy-src}/examples/authentication/password/authentication.json
        test -e ${cowboy-src}/examples/authentication/passkey/plugin.json
        test -e ${cowboy-src}/examples/authentication/passkey/authentication.json
        test -e ${cowboy-src}/examples/authentication/google/host.json
        test ! -e ${cowboy-src}/examples/authentication/google/ui/index.js
        test ! -e ${cowboy-src}/examples/authentication/README.md
        test ! -e ${cowboy-src}/components/provider-runtime/lock.json
        test -e ${machine-src}/components/provider-sdk/Cargo.toml
        test -e ${machine-src}/components/plugin-sdk/Cargo.toml
        test -e ${machine-src}/build.rs
        test -e ${machine-src}/src/first_party_sources.rs
        test -e ${machine-src}/src/plugin_process.rs
        test -e ${machine-src}/src/code_buffer_read/content.rs
        test -e ${machine-src}/src/code_buffer_read/text.rs
        test -e ${machine-src}/plugins/zed/adapter/fixtures/content.json
        test -e ${machine-src}/plugins/zed/adapter/fixtures/text.json
        test ! -e ${machine-src}/plugins/zed/adapter/src/main.rs
        test ! -e ${machine-src}/contracts/code-buffer-sync.fixture.json
        test ! -e ${machine-src}/contracts/code-buffer-sync-budget.fixture.json
        test ! -e ${machine-src}/contracts/code-buffer-navigation.fixture.json
        test ! -e ${machine-src}/contracts/code-buffer-destination.fixture.json
        test -e ${machine-src}/plugins/gemini/provider.json
        test ! -e ${machine-src}/components/provider-runtime/lock.json
        test -e ${code-adapter-src}/components/provider-sdk/Cargo.toml
        test -e ${code-adapter-src}/components/plugin-sdk/Cargo.toml
        test ! -e ${code-adapter-src}/providers
        test -e ${machine-src}/src/provider/deepseek_cache.rs
        test -e ${machine-src}/src/plugin_runtime_args.rs
        test -e ${machine-src}/plugins/grok/host.json
        test -e ${machine-src}/src/provider/deepseek_context.rs
        test -e ${machine-src}/src/machine_plugins.rs
        test -e ${machine-src}/src/machine_plugins/operations.rs
        test -e ${machine-src}/src/machine_plugins/operations/installations.rs
        test -e ${machine-src}/src/machine_plugins/operations/lease.rs
        test -e ${machine-src}/src/operation_budget.rs
        test -e ${machine-src}/src/owned_json.rs
        test -e ${machine-src}/src/machine_protocol/plugin_step.rs
        test -e ${machine-src}/src/machine_protocol/plugin_recovery.rs
        test -e ${machine-src}/src/machine_protocol/installation_revision.rs
        test -e ${machine-src}/src/provider_behavior.rs
        test -e ${machine-src}/src/provider_catalog.rs
        test -e ${machine-src}/src/provider_usage_spool.rs
        test -e ${machine-src}/src/session_workspace.rs
        test -e ${machine-src}/src/machine_broker/deletions.rs
        test -e ${machine-src}/src/session_deletion_admission.rs
        test -e ${machine-src}/src/machine_install/bootstrap_probe.rs
        test ! -e ${cowboy}/bin/cowboy-machine
        test ! -e ${cowboy}/bin/cowboy-machine-install
        test -x ${cowboy}/bin/cowboy-codex-app-server
        test -x ${cowboy}/bin/cowboy-plugin-js
        test -x ${cowboy-machine}/bin/cowboy-machine
        test -x ${cowboy-machine}/bin/cowboy-machine-install
        test -x ${cowboy-machine}/bin/cowboy
        test -x ${cowboy-machine}/bin/cowboy-plugin-js
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
        cowboy-execution-runtime = execution-runtime;
        default = cowboy;
        cowboy = cowboy;
        cowboy-machine = cowboy-machine;
        cowboy-code-adapter = cowboy-code-adapter;
        cowboy-plugin-pack = cowboy-plugin-pack;
        cowboy-zed-adapter = cowboy-zed-adapter;
        cowboy-zed-adapter-portable = cowboy-zed-adapter-portable;
        cowboy-zed-server = cowboy-zed-server;
        cowboy-web = cowboy-web;
        # Optional local conformance tool, never part of a product runtime.
        cowboy-idb-test-browser = pkgs.firefox;
        cowboy-controller-release = cowboy-controller-release;
        cowboy-web-release = cowboy-web-release;
        cowboy-machine-bootstrap-release = cowboy-machine-bootstrap-release;
        cowboy-machine-release = cowboy-machine-release;
        cowboy-machine-host-release = cowboy-machine-host-release;
      };

      # `cowboy`'s buildRustPackage check phase runs the Rust tests; cowboy-web's
      # build runs TypeScript checking before Vite. Developer lint/test policy is
      # additionally enforced by `just check` in CI.
      checks.${system} = {
        inherit cowboy cowboy-machine cowboy-code-adapter cowboy-source-boundary
          cowboy-web cowboy-controller-release cowboy-web-release
          cowboy-machine-bootstrap-release cowboy-machine-release
          cowboy-machine-host-release
          cowboy-zed-integration cowboy-zed-adapter cowboy-zed-server;
      };

      # Android native-shell builds on Linux. The Rust and Tauri CLI versions
      # come from apps/native-shell/toolchain.json, so the shell cannot drift
      # from the Apple pins. The Android SDK/NDK stay owned by Android Studio's
      # SDK Manager (ANDROID_HOME, NDK_HOME); the builder verifies their exact
      # versions instead of vendoring them into Nix.
      devShells.${system} = {
      native-android = let
        nativeToolchain = builtins.fromJSON
          (builtins.readFile ./apps/native-shell/toolchain.json);
        androidRust = pkgs.rust-bin.stable.${nativeToolchain.rust}.minimal.override {
          targets = map (abi: abi.rustTarget) nativeToolchain.android.abis;
        };
        # nixpkgs trails the pinned CLI; build the exact crates.io release with
        # its published lockfile. Update both hashes with the toolchain pin.
        # crates.io's download API rejects Nix fetchers (see
        # staticCratesVendorPatch), so read the immutable static CDN directly.
        tauriCliSrc = pkgs.runCommand "tauri-cli-${nativeToolchain.tauriCli}-source" {
          crate = pkgs.fetchurl {
            url = "https://static.crates.io/crates/tauri-cli/tauri-cli-${nativeToolchain.tauriCli}.crate";
            hash = "sha256-LaQKekLq4/dOtHg+eDeODYzoSsgljhwSNEN1VgnNMnc=";
          };
        } ''
          mkdir "$out"
          tar -xzf "$crate" -C "$out" --strip-components=1
        '';
        tauriCli = rustPlatform.buildRustPackage {
          pname = "tauri-cli";
          version = nativeToolchain.tauriCli;
          src = tauriCliSrc;
          cargoDeps = rustPlatform.fetchCargoVendor {
            pname = "tauri-cli";
            version = nativeToolchain.tauriCli;
            src = tauriCliSrc;
            hash = "sha256-3ZKnrr6fKS1xmBoNlkh4WhXQHoV8DmxCRRZirN0hUrg=";
            preBuild = staticCratesVendorPatch;
          };
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.openssl pkgs.bzip2 pkgs.xz pkgs.zstd ];
          doCheck = false;
        };
      in pkgs.mkShell {
        COWBOY_NATIVE_ANDROID_SHELL = "1";
        JAVA_HOME = "${pkgs.jdk21}/lib/openjdk";
        nativeBuildInputs = [
          androidRust
          tauriCli
          pkgs.jdk21
          deno
          pkgs.python3
          pkgs.git
          pkgs.gnutar
          pkgs.just
          pkgs.jq
          pkgs.unzip
        ];
        shellHook = ''
          echo "cowboy native-android shell — $(rustc --version), $(cargo tauri --version)"
        '';
      };

      default = pkgs.mkShell {
        # See the controller package note above. This only affects C build
        # scripts; rustc keeps the profile selected by Cargo.
        CFLAGS = "-O1";
        # Rust toolchain plus opt-in sccache, and the frontend toolchain
        # (Cowboy's pinned Deno + node 24 for any node-shaped tool that
        # deno's npm interop can't shim).
        COWBOY_DENO_VERSION = deno.version;
        COWBOY_NODE_VERSION = cowboy-nodejs.version;
        nativeBuildInputs = with pkgs; [
          rustToolchain
          sccache
          cargo-nextest
          cargo-deny
          cargo-machete
          brotli
          curl
          git
          nix
          gnutar
          gzip
          just
          jq
          go
          imagemagick
          python3
          util-linux
          iproute2
          # `assert_process_stopped` reads process state with `ps`. A host that
          # happens to have procps on PATH hid this; the pinned shell must
          # declare it or four owned-runtime teardown tests abort on ENOENT.
          procps
          # Ephemeral, socket-only database for the PostgreSQL contract gate.
          # This is a developer/test dependency, not a Controller runtime input.
          (lib.getBin postgresql)
        ] ++ [ deno cowboy-nodejs ];

        shellHook = ''
          echo "cowboy dev shell — rust + optional sccache + deno"
          sccache --version >/dev/null 2>&1 && echo "sccache: $(sccache --version)"
          deno --version 2>/dev/null | head -1
        '';
      };
      };
    };
}
