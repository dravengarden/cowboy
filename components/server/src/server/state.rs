//! Controller services, including independent memory and MCP policy owners.
use super::*;

#[derive(Default)]
pub(super) struct SessionExtensions {
    pub(super) memory: memory_relay::Relay,
    pub(super) bindings: mcp_bindings::Policies,
    pub(super) launches: mcp_bindings::launch::Launches,
    pub(super) mcp: mcp_bindings::relay::Relay,
    pub(super) apps: mcp_bindings::apps::Frames,
}

pub(super) struct AppState {
    pub(super) service_id: String,
    pub(super) service_config_path: PathBuf,
    /// The Service configuration file, with live fields following edits.
    pub(super) service_config: crate::config::Handle,
    pub(super) hub: Hub,
    pub(super) supervisor: Arc<Supervisor>,
    /// Kept for read-only storage metrics (`/api/metrics`). `None` in-memory.
    pub(super) store: Option<Store>,
    pub(super) persistence_health: Option<Arc<PersistenceHealth>>,
    pub(super) shutdown: watch::Receiver<bool>,
    pub(super) runtime_health: Arc<RuntimeHealth>,
    pub(super) runtime_router: Arc<RuntimeRouter>,
    pub(super) machine_control: Arc<MachineControl>,
    pub(super) machine_snapshots: MachineSnapshots,
    pub(super) session_maintenance: session_maintenance::Coordinator,
    pub(super) browsers: provider_accounts::browser::coordinator::Coordinator,
    pub(super) plugin_catalog: Arc<crate::plugin_catalog::PluginCatalog>,
    pub(super) plugin_storage: crate::plugin_storage::PluginStorage,
    pub(super) provider_catalog: Arc<crate::provider_catalog::ProviderCatalog>,
    pub(super) provider_auth: Arc<crate::provider_service::ProviderAuthService>,
    pub(super) provider_auth_sync: provider_auth_sync::Coordinator,
    pub(super) provider_auth_executors: parking_lot::Mutex<HashMap<String, ProviderAuthExecutor>>,
    pub(super) plugin_uninstall_plans: parking_lot::Mutex<HashMap<String, PluginUninstallPlan>>,
    pub(super) plugin_resolution_plans: plugin_uninstall::resolution::ResolutionPlans,
    pub(super) telemetry_resolution_plans: Arc<telemetry_binding::resolution::surface::Plans>,
    pub(super) telemetry_recovery_plans: Arc<telemetry_binding::recovery::surface::Plans>,
    pub(super) telemetry_binding_plans: Arc<telemetry_binding::surface::Plans>,
    pub(super) telemetry_binding_fence: crate::telemetry_binding::LegacyFence,
    pub(super) telemetry_writer_admission:
        Option<Arc<crate::telemetry_plugin::writer_admission::WriterAdmission>>,
    pub(super) plugin_lifecycle_fences: PluginLifecycleFences,
    pub(super) session_create_replays: Arc<session_create_replay::Replays>,
    pub(super) desired_machine_components: Arc<crate::machine_convergence::DesiredComponentSource>,
    pub(super) component_convergence: Arc<crate::machine_convergence::ConvergenceState>,
    pub(super) convergence_freeze: Arc<crate::machine_convergence::ConvergenceFreeze>,
    pub(super) desired_machine_plugins: Arc<crate::machine_convergence::PluginDesiredSource>,
    pub(super) web_root: PathBuf,
    pub(super) web_roots: web_roots::WebRoots,
    pub(super) usage: UsageService,
    pub(super) diff_snapshots: DiffSnapshotCache,
    pub(super) file_cursors: code_reads::file_pages::PageCursors,
    pub(super) code_cache: crate::code_cache::CodeCache,
    pub(super) code_buffers: Arc<code_buffers::Owners>,
    pub(super) code_navigation_admission: bool,
    pub(super) project_placement: crate::project_placement::Store,
    pub(super) execution_preparations: parking_lot::Mutex<std::collections::HashSet<String>>,
    pub(super) workspace_preparations: parking_lot::Mutex<std::collections::HashSet<String>>,
    pub(super) execution_readiness: execution::recovery::Readiness,
    pub(super) lost_turns: execution::recovery::LostTurns,
    pub(super) execution_closures:
        Arc<parking_lot::Mutex<std::collections::HashMap<String, execution::Maintenance>>>,
    pub(super) managed_calls: Arc<managed_calls::Coordinator>,
    pub(super) extensions: SessionExtensions,
    /// Machines permitted to have their Code reads executed on the
    /// Controller's own filesystem. A hello's declared connection mode is a
    /// request; this set is the permission.
    pub(super) colocated_machines: std::collections::BTreeSet<String>,
    pub(super) zed_adapter_socket: Option<PathBuf>,
    pub(super) observability: Observability,
    pub(super) web_push: Arc<WebPushService>,
    pub(super) notifications: notifications::Presence,
    pub(super) public_origins: Arc<Vec<String>>,
    pub(super) product_auth_enabled: bool,
    pub(super) product_authentication: Arc<crate::auth_plugins::ProductAuthentication>,
    pub(super) oidc_transactions: Arc<crate::oidc::OidcTransactions>,
    pub(super) oidc_native_handoffs: Arc<crate::oidc::NativeHandoffs>,
    pub(super) device_authorizations: Arc<crate::client_auth::DeviceAuthorizations>,
    pub(super) device_access: Arc<crate::client_auth::DeviceAccessSessions>,
    pub(super) capacity_runtime: Arc<ActiveCapacityRuntime>,
}

impl AppState {
    pub(super) fn memory_relay(&self) -> &memory_relay::Relay {
        &self.extensions.memory
    }
}
