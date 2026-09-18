{ pkgs, rustToolchain, rustPlatform }:
# Private server source build. Building is neither publication nor installation;
# exact static server/adapter bytes require connected capability acceptance.
let
  src = pkgs.fetchFromGitHub {
    owner = "zed-industries";
    repo = "zed";
    rev = "aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45";
    hash = "sha256-sAjiYGwmQB+Zzb/b7PGm4Nfv36Vb0myqKIBfpuHGTik=";
  };
  toolchain = rustToolchain.override {
    targets = [ "x86_64-unknown-linux-musl" ];
  };
  platform = pkgs.pkgsCross.musl64.makeRustPlatform {
    cargo = toolchain;
    rustc = toolchain;
  };
  cargoDeps = rustPlatform.fetchCargoVendor {
    pname = "cowboy-private-zed-server";
    version = "1.13.0";
    inherit src;
    patches = [ ./server/dependencies.patch ];
    hash = "sha256-lWM3FdtmfZdYfN/zSFao8713Xtkw9c7RyLsSLOH3RfM=";
    # Same checksum-preserving immutable CDN repair as Cowboy's core builder.
    preBuild = ''
      vendor_util="$(command -v fetch-cargo-vendor-util-v2 || command -v fetch-cargo-vendor-util)"
      if grep -q "https://crates.io/api/v1/crates/" "$vendor_util"; then
        patched_util="$TMPDIR/cargo-vendor-bin/$(basename "$vendor_util")"
        mkdir -p "$(dirname "$patched_util")"
        cp "$vendor_util" "$patched_util"
        chmod u+w "$patched_util"
        substituteInPlace "$patched_util" \
          --replace-fail "https://crates.io/api/v1/crates/" \
            "https://static.crates.io/crates/"
        export PATH="$(dirname "$patched_util"):$PATH"
      fi
    '';
  };
in platform.buildRustPackage {
  pname = "cowboy-zed-server";
  # Private distribution version, not a claim to be an upstream Zed release.
  version = "1.3.0";
  inherit src cargoDeps;
  patches = [ ./server/dependencies.patch ./server/input-bounds.patch ./server/reload-bounds.patch ./server/acquisition-bounds.patch ];
  postPatch = ''
    cp ${../adapter/proto/cowboy-buffer.proto} crates/proto/proto/cowboy-buffer.proto
    mkdir -p crates/project/src/buffer_store/cowboy_sync
    cp ${./server/cowboy_sync.rs} crates/project/src/buffer_store/cowboy_sync.rs
    cp ${./server/tests.rs} crates/project/src/buffer_store/cowboy_sync/tests.rs
    mkdir -p crates/project/src/buffer_store/cowboy_close
    cp ${./server/cowboy_close.rs} crates/project/src/buffer_store/cowboy_close.rs
    cp ${./server/close_tests.rs} crates/project/src/buffer_store/cowboy_close/tests.rs
    cp ${./server/reload_tests.rs} crates/project/src/buffer_store/cowboy_reload_tests.rs
    cp ${./server/acquisition_tests.rs} crates/project/src/buffer_store/cowboy_acquisition_tests.rs
    cp ${./server/cowboy_buffer_budget.rs} crates/language/src/cowboy_buffer_budget.rs
    cp ${./server/cowboy_bounded.rs} crates/fs/src/cowboy_bounded.rs
    cp ${./server/cowboy_lsp_input.rs} crates/lsp/src/cowboy_lsp_input.rs
    mkdir -p crates/project/src/lsp_store/cowboy_navigation
    cp ${./server/cowboy_navigation.rs} crates/project/src/lsp_store/cowboy_navigation.rs
    cp ${./server/navigation_tests.rs} crates/project/src/lsp_store/cowboy_navigation/tests.rs
    substituteInPlace crates/proto/proto/zed.proto \
      --replace-fail 'import "buffer.proto";' 'import "buffer.proto";
    import "cowboy-buffer.proto";' \
      --replace-fail 'oneof payload {' 'oneof payload {
        CowboyBufferSync cowboy_buffer_sync = 1000;
        CowboyBufferSyncResponse cowboy_buffer_sync_response = 1001;
        CowboyNavigation cowboy_navigation = 1002;
        CowboyNavigationResponse cowboy_navigation_response = 1003;
        CowboyCloseBuffers cowboy_close_buffers = 1004;
        CowboyCloseBuffersResponse cowboy_close_buffers_response = 1005;'
    substituteInPlace crates/proto/src/proto.rs \
      --replace-fail '(ReloadBuffers, Foreground),' '(ReloadBuffers, Foreground),
        (CowboyBufferSync, Foreground),
        (CowboyBufferSyncResponse, Foreground),
        (CowboyNavigation, Foreground),
        (CowboyNavigationResponse, Foreground),
        (CowboyCloseBuffers, Foreground),
        (CowboyCloseBuffersResponse, Foreground),' \
      --replace-fail '(ReloadBuffers, ReloadBuffersResponse),' '(ReloadBuffers, ReloadBuffersResponse),
        (CowboyBufferSync, CowboyBufferSyncResponse),
        (CowboyNavigation, CowboyNavigationResponse),
        (CowboyCloseBuffers, CowboyCloseBuffersResponse),' \
      --replace-fail '    ReloadBuffers,' '    ReloadBuffers,
        CowboyBufferSync,
        CowboyNavigation,
        CowboyCloseBuffers,'
    substituteInPlace crates/project/src/lsp_store.rs \
      --replace-fail 'pub struct LspStore {' 'mod cowboy_navigation;
    pub struct LspStore {
        cowboy_navigation: cowboy_navigation::State,' \
      --replace-fail 'next_hint_id: Arc::default(),' 'next_hint_id: Arc::default(),
            cowboy_navigation: Default::default(),' \
      --replace-fail 'client.add_entity_request_handler(Self::handle_lsp_query);' 'client.add_entity_request_handler(Self::handle_lsp_query);
        client.add_entity_request_handler(Self::handle_cowboy_navigation);'
    substituteInPlace crates/project/src/buffer_store.rs \
      --replace-fail '/// A set of open buffers.' 'mod cowboy_sync;
    mod cowboy_close;
    #[cfg(test)]
    #[path = "buffer_store/cowboy_reload_tests.rs"]
    mod cowboy_reload_tests;
    #[cfg(test)]
    #[path = "buffer_store/cowboy_acquisition_tests.rs"]
    mod cowboy_acquisition_tests;
    /// A set of open buffers.' \
      --replace-fail 'pub struct BufferStore {' 'pub struct BufferStore {
        cowboy_sync: cowboy_sync::State,
        cowboy_close: cowboy_close::State,' \
      --replace-fail 'project_search: Default::default(),' 'project_search: Default::default(),
            cowboy_sync: Default::default(),
            cowboy_close: Default::default(),' \
      --replace-fail 'client.add_entity_request_handler(Self::handle_reload_buffers);' 'client.add_entity_request_handler(Self::handle_reload_buffers);
        client.add_entity_request_handler(Self::handle_cowboy_buffer_sync);
        client.add_entity_request_handler(Self::handle_cowboy_close_buffers);' \
      --replace-fail '    pub fn has_shared_buffers(&self) -> bool {' '    pub(crate) fn cowboy_navigation_owner(&self, peer: proto::PeerId, buffer: &Entity<Buffer>, cx: &App) -> bool {
            self.shared_buffers.get(&peer)
                .and_then(|values| values.get(&buffer.read(cx).remote_id()))
                .is_some_and(|shared| shared.buffer == *buffer)
        }

        pub fn has_shared_buffers(&self) -> bool {'
    substituteInPlace crates/language/src/buffer.rs \
      --replace-fail '    /// Reloads the contents of the buffer from disk.' '    /// Private Cowboy conditional-sync admission; caller rechecks in the mutation turn.
        pub fn cowboy_can_sync(&self) -> bool {
            self.capability == Capability::ReadWrite && !self.is_dirty()
                && !self.has_conflict && self.reload_task.is_none()
                && self.encoding == encoding_rs::UTF_8 && !self.has_bom
                && self.line_ending() == LineEnding::Unix
        }

        /// Reloads the contents of the buffer from disk.'
    substituteInPlace crates/fs/src/fs.rs \
      --replace-fail 'pub mod fs_watcher;' 'pub mod fs_watcher;
    mod cowboy_bounded;' \
      --replace-fail '    async fn open_handle(&self, path: &Path) -> Result<Arc<dyn FileHandle>>;' '    async fn cowboy_load_bytes_bounded(&self, path: &Path, limit: usize) -> Result<Vec<u8>> {
            cowboy_bounded::load(self, path, limit).await
        }
        async fn open_handle(&self, path: &Path) -> Result<Arc<dyn FileHandle>>;' \
      --replace-fail 'impl Fs for RealFs {' 'impl Fs for RealFs {
        async fn cowboy_load_bytes_bounded(&self, path: &Path, limit: usize) -> Result<Vec<u8>> {
            cowboy_bounded::load_real(path, limit)
        }'
  '';
  # Follow upstream's independent remote_server invocation: editor feature
  # unification would introduce dynamic desktop dependencies into this target.
  cargoBuildFlags = [ "--package" "remote_server" "--bin" "remote_server" ];
  doCheck = true;
  cargoTestFlags = [ "--package" "project" "--package" "fs" "--package" "lsp" "--lib"
    "--features" "project/test-support,fs/test-support,lsp/test-support" "cowboy_" ];
  nativeBuildInputs = [ pkgs.cmake pkgs.perl pkgs.pkg-config pkgs.protobuf rustPlatform.bindgenHook ];
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER = "${pkgs.stdenv.cc}/bin/cc";
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS = "--cfg tokio_unstable -C target-feature=+crt-static -C force-frame-pointers=yes";
  installPhase = ''
    runHook preInstall
    install -Dm555 target/x86_64-unknown-linux-musl/release/remote_server "$out/bin/cowboy-zed-server"
    install -Dm444 LICENSE-GPL "$out/share/licenses/cowboy-zed-server/LICENSE-GPL"
    runHook postInstall
  '';
  postFixup = ''
    if ${pkgs.binutils}/bin/readelf -l "$out/bin/cowboy-zed-server" | ${pkgs.gnugrep}/bin/grep -q INTERP; then
      echo 'private Zed server unexpectedly needs a dynamic loader' >&2
      exit 1
    fi
    if ${pkgs.binutils}/bin/readelf -d "$out/bin/cowboy-zed-server" | ${pkgs.gnugrep}/bin/grep -q NEEDED; then
      echo 'private Zed server unexpectedly needs shared libraries' >&2
      exit 1
    fi
  '';
  meta = {
    description = "Private pinned Zed server candidate for conditional buffer synchronization";
    license = pkgs.lib.licenses.gpl3Plus;
    mainProgram = "cowboy-zed-server";
  };
}
