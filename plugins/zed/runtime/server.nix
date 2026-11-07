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
    # Dependency snapshot is unchanged by the private journal protocol update.
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
  version = "1.15.0";
  inherit src cargoDeps;
  patches = [ ./server/dependencies.patch ./server/input-bounds.patch ./server/reload-bounds.patch ./server/acquisition-bounds.patch ./server/replacement-bounds.patch ./server/snapshot-lifetimes.patch ./server/remote-edits.patch ./server/worktree-attempts.patch ];
  postPatch = ''
    cp ${../adapter/proto/cowboy-buffer.proto} crates/proto/proto/cowboy-buffer.proto
    mkdir -p crates/project/src/buffer_store/cowboy_sync
    cp ${./server/cowboy_sync.rs} crates/project/src/buffer_store/cowboy_sync.rs
    cp ${./server/tests.rs} crates/project/src/buffer_store/cowboy_sync/tests.rs
    mkdir -p crates/project/src/buffer_store/cowboy_close
    cp ${./server/cowboy_close.rs} crates/project/src/buffer_store/cowboy_close.rs
    cp ${./server/close_tests.rs} crates/project/src/buffer_store/cowboy_close/tests.rs
    mkdir -p crates/project/src/buffer_store/cowboy_open_attempts
    cp ${./server/cowboy_open_attempts.rs} crates/project/src/buffer_store/cowboy_open_attempts.rs
    cp ${./server/open_attempts_tests.rs} crates/project/src/buffer_store/cowboy_open_attempts/tests.rs
    mkdir -p crates/project/src/buffer_store/cowboy_worktree_attempts
    cp ${./server/cowboy_worktree_attempts.rs} crates/project/src/buffer_store/cowboy_worktree_attempts.rs
    cp ${./server/worktree_attempts_tests.rs} crates/project/src/buffer_store/cowboy_worktree_attempts/tests.rs
    cp ${./server/reload_tests.rs} crates/project/src/buffer_store/cowboy_reload_tests.rs
    cp ${./server/acquisition_tests.rs} crates/project/src/buffer_store/cowboy_acquisition_tests.rs
    cp ${./server/cowboy_buffer_budget.rs} crates/language/src/cowboy_buffer_budget.rs
    cp ${./server/cowboy_acquisition.rs} crates/text/src/cowboy_acquisition.rs
    cp ${./server/cowboy_full_offsets.rs} crates/text/src/cowboy_full_offsets.rs
    cp ${./server/cowboy_replacement.rs} crates/language/src/buffer/cowboy_replacement.rs
    cp ${./server/cowboy_remote_edits.rs} crates/language/src/buffer/cowboy_remote_edits.rs
    cp ${./server/replacement_tests.rs} crates/project/src/buffer_store/cowboy_replacement_tests.rs
    cp ${./server/remote_edits_tests.rs} crates/project/src/buffer_store/cowboy_remote_edits_tests.rs
    cp ${./server/cowboy_bounded.rs} crates/fs/src/cowboy_bounded.rs
    cp ${./server/cowboy_lsp_input.rs} crates/lsp/src/cowboy_lsp_input.rs
    mkdir -p crates/project/src/lsp_store/cowboy_navigation
    cp ${./server/cowboy_navigation.rs} crates/project/src/lsp_store/cowboy_navigation.rs
    cp ${./server/navigation_tests.rs} crates/project/src/lsp_store/cowboy_navigation/tests.rs
    cp ${./server/navigation_receipts.rs} crates/project/src/lsp_store/navigation_receipts.rs
    mkdir -p crates/project/src/lsp_store/cowboy_frontier
    cp ${./server/cowboy_frontier.rs} crates/project/src/lsp_store/cowboy_frontier.rs
    cp ${./server/frontier_tests.rs} crates/project/src/lsp_store/cowboy_frontier/tests.rs
    substituteInPlace crates/proto/proto/zed.proto \
      --replace-fail 'import "buffer.proto";' 'import "buffer.proto";
    import "cowboy-buffer.proto";' \
      --replace-fail 'oneof payload {' 'oneof payload {
        CowboyBufferSync cowboy_buffer_sync = 1000;
        CowboyBufferSyncResponse cowboy_buffer_sync_response = 1001;
        CowboyNavigation cowboy_navigation = 1002;
        CowboyNavigationResponse cowboy_navigation_response = 1003;
        CowboyCloseBuffers cowboy_close_buffers = 1004;
        CowboyCloseBuffersResponse cowboy_close_buffers_response = 1005;
        CowboyAnalysisFrontier cowboy_analysis_frontier = 1006;
        CowboyAnalysisFrontierResponse cowboy_analysis_frontier_response = 1007;
        CowboyOpenAttempt cowboy_open_attempt = 1008;
        CowboyOpenAttemptResponse cowboy_open_attempt_response = 1009;
        CowboyOpenTarget cowboy_open_target = 1010;
        CowboyOpenTargetResponse cowboy_open_target_response = 1011;
        CowboyWorktreeAttempt cowboy_worktree_attempt = 1012;
        CowboyWorktreeAttemptResponse cowboy_worktree_attempt_response = 1013;'
    substituteInPlace crates/proto/src/proto.rs \
      --replace-fail '(ReloadBuffers, Foreground),' '(ReloadBuffers, Foreground),
        (CowboyBufferSync, Foreground),
        (CowboyBufferSyncResponse, Foreground),
        (CowboyNavigation, Foreground),
        (CowboyNavigationResponse, Foreground),
        (CowboyCloseBuffers, Foreground),
        (CowboyCloseBuffersResponse, Foreground),
        (CowboyAnalysisFrontier, Foreground),
        (CowboyAnalysisFrontierResponse, Foreground),
        (CowboyOpenAttempt, Foreground),
        (CowboyOpenAttemptResponse, Foreground),
        (CowboyOpenTarget, Foreground),
        (CowboyOpenTargetResponse, Foreground),
        (CowboyWorktreeAttempt, Foreground),
        (CowboyWorktreeAttemptResponse, Foreground),' \
      --replace-fail '(ReloadBuffers, ReloadBuffersResponse),' '(ReloadBuffers, ReloadBuffersResponse),
        (CowboyBufferSync, CowboyBufferSyncResponse),
        (CowboyNavigation, CowboyNavigationResponse),
        (CowboyCloseBuffers, CowboyCloseBuffersResponse),
        (CowboyAnalysisFrontier, CowboyAnalysisFrontierResponse),
        (CowboyOpenAttempt, CowboyOpenAttemptResponse),
        (CowboyOpenTarget, CowboyOpenTargetResponse),
        (CowboyWorktreeAttempt, CowboyWorktreeAttemptResponse),' \
      --replace-fail '    ReloadBuffers,' '    ReloadBuffers,
        CowboyBufferSync,
        CowboyNavigation,
        CowboyCloseBuffers,
        CowboyAnalysisFrontier,
        CowboyOpenAttempt,
        CowboyOpenTarget,
        CowboyWorktreeAttempt,'
    substituteInPlace crates/project/src/lsp_store.rs \
      --replace-fail '                version: 0,
                    snapshot: initial_snapshot.clone(),' '                version: 1,
                    snapshot: initial_snapshot.clone(),' \
      --replace-fail '                        adapter.language_id(&language.name()),
                            0,
                            initial_snapshot.text(),' '                        adapter.language_id(&language.name()),
                            1,
                            initial_snapshot.text(),' \
      --replace-fail '                                version: 0,
                                    snapshot: buffer.text_snapshot(),' '                                version: 1,
                                    snapshot: buffer.text_snapshot(),' \
      --replace-fail 'pub struct LspStore {' 'mod cowboy_navigation;
    mod cowboy_frontier;
    pub struct LspStore {
        cowboy_navigation: cowboy_navigation::State,' \
      --replace-fail 'next_hint_id: Arc::default(),' 'next_hint_id: Arc::default(),
            cowboy_navigation: Default::default(),' \
      --replace-fail 'client.add_entity_request_handler(Self::handle_lsp_query);' 'client.add_entity_request_handler(Self::handle_lsp_query);
        client.add_entity_request_handler(Self::handle_cowboy_navigation);
        client.add_entity_request_handler(Self::handle_cowboy_analysis_frontier);' \
      --replace-fail '    buffer_snapshots: HashMap<BufferId, HashMap<LanguageServerId, Vec<LspBufferSnapshot>>>, // buffer_id -> server_id -> vec of snapshots' '    buffer_snapshots: HashMap<BufferId, HashMap<LanguageServerId, Vec<LspBufferSnapshot>>>, // buffer_id -> server_id -> vec of snapshots
        cowboy_frontier: cowboy_frontier::State,' \
      --replace-fail '                buffer_snapshots: Default::default(),' '                buffer_snapshots: Default::default(),
                    cowboy_frontier: Default::default(),' \
      --replace-fail '        let diagnostics = self.pull_diagnostics(buffer, cx);
            cx.spawn(async move |lsp_store, cx| {
                let diagnostics = match diagnostics.await {' '        let cowboy_versions = self.cowboy_pull_versions(&buffer, cx);
            let diagnostics = self.pull_diagnostics(buffer, cx);
            cx.spawn(async move |lsp_store, cx| {
                let diagnostics = match diagnostics.await {' \
      --replace-fail '                                        uri,
                                            diagnostics,
                                            version: None,
                                        },
                                        result_id: result_id.map(SharedString::new),' '                                        uri,
                                            diagnostics,
                                            // Cowboy: the version the pull was answered for.
                                            version: cowboy_versions.get(&server_id).copied(),
                                        },
                                        result_id: result_id.map(SharedString::new),' \
      --replace-fail '        let snapshot = self.buffer_snapshot_for_lsp_version(buffer, server_id, version, cx)?;

            let edits_since_save' '        let snapshot = self.buffer_snapshot_for_lsp_version(buffer, server_id, version, cx)?;
            let cowboy_evidence = (
                cowboy_frontier::tracked_version(self, buffer, server_id, version, cx),
                snapshot.version().clone(),
                diagnostics.iter().any(|(new, entry)| *new && entry.diagnostic.is_disk_based),
                diagnostics.iter().any(|(new, _)| !*new),
            );

            let edits_since_save' \
      --replace-fail '            buffer.update_diagnostics(server_id, set, cx)
            });

            Ok(())
        }

        fn register_language_server_for_invisible_worktree(' '            buffer.update_diagnostics(server_id, set, cx);
                let (versioned, published, disk_based, reused) = cowboy_evidence;
                self.cowboy_frontier.record(
                    buffer.remote_id(),
                    server_id,
                    cowboy_frontier::Evidence {
                        stamp: buffer.cowboy_diagnostics_timestamp(),
                        versioned,
                        published,
                        saved: buffer.saved_version().clone(),
                        disk_based,
                        reused,
                    },
                );
            });

            Ok(())
        }

        fn register_language_server_for_invisible_worktree('
    substituteInPlace crates/project/src/buffer_store.rs \
      --replace-fail '/// A set of open buffers.' 'mod cowboy_sync;
    mod cowboy_close;
    pub mod cowboy_open_attempts;
    pub mod cowboy_worktree_attempts;
    #[cfg(test)]
    #[path = "buffer_store/cowboy_reload_tests.rs"]
    mod cowboy_reload_tests;
    #[cfg(test)]
    #[path = "buffer_store/cowboy_acquisition_tests.rs"]
    mod cowboy_acquisition_tests;
    #[cfg(test)]
    #[path = "buffer_store/cowboy_replacement_tests.rs"]
    mod cowboy_replacement_tests;
    #[cfg(test)]
    #[path = "buffer_store/cowboy_remote_edits_tests.rs"]
    mod cowboy_remote_edits_tests;
    /// A set of open buffers.' \
      --replace-fail 'pub struct BufferStore {' 'pub struct BufferStore {
        cowboy_sync: cowboy_sync::State,
        cowboy_close: cowboy_close::State,
        cowboy_open_attempts: cowboy_open_attempts::State,
        cowboy_worktree_attempts: cowboy_worktree_attempts::State,' \
      --replace-fail 'project_search: Default::default(),' 'project_search: Default::default(),
            cowboy_sync: Default::default(),
            cowboy_close: Default::default(),
            cowboy_open_attempts: Default::default(),
            cowboy_worktree_attempts: Default::default(),' \
      --replace-fail 'client.add_entity_request_handler(Self::handle_reload_buffers);' 'client.add_entity_request_handler(Self::handle_reload_buffers);
        client.add_entity_request_handler(Self::handle_cowboy_buffer_sync);
        client.add_entity_request_handler(Self::handle_cowboy_close_buffers);
        client.add_entity_request_handler(Self::handle_cowboy_open_attempt);
        client.add_entity_request_handler(Self::handle_cowboy_open_target);
        client.add_entity_request_handler(Self::handle_cowboy_worktree_attempt);' \
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

        /// Private Cowboy analysis evidence: the newest diagnostics operation.
        pub fn cowboy_diagnostics_timestamp(&self) -> clock::Lamport {
            self.diagnostics_timestamp
        }

        /// Reloads the contents of the buffer from disk.'
    substituteInPlace crates/remote_server/src/headless_project.rs \
      --replace-fail '    ) -> Result<proto::OpenBufferResponse> {
            let worktree_id = WorktreeId::from_proto(message.payload.worktree_id);
            let path = RelPath::from_unix_str(&message.payload.path)?.into();' '    ) -> Result<proto::OpenBufferResponse> {
            // Recorded before the first await so a peer can learn the outcome.
            let cowboy_attempt = this.read_with(&cx, |this, cx| {
                this.buffer_store
                    .read(cx)
                    .cowboy_open_attempts()
                    .begin(message.sender_id, message.message_id)
            })?;
            let worktree_id = WorktreeId::from_proto(message.payload.worktree_id);
            let path = RelPath::from_unix_str(&message.payload.path)?.into();' \
      --replace-fail 'buffer_store.open_buffer(ProjectPath { worktree_id, path }, cx)
                });
                (buffer_store, buffer)
            });

            let buffer = buffer.await?;
            let buffer_id = buffer.read_with(&cx, |b, _| b.remote_id());' 'buffer_store.open_buffer(ProjectPath { worktree_id, path }, cx)
                });
                (buffer_store, buffer)
            });

            let buffer = buffer.await?;
            let buffer_id = buffer.read_with(&cx, |b, _| b.remote_id());
            cowboy_attempt.opened(buffer_id.to_proto());'
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
