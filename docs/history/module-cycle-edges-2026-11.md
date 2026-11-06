# Base-crate module cycle: edge inventory (2026-11-06)

Plan slice 5.1 of [`structural-refactor-plan-2026-11.md`](../structural-refactor-plan-2026-11.md).
The largest recorded cycle in `tools/module-edges-baseline.json` has 37 modules of the
`cowboy` crate and 126 edges between them. Each edge A → B was classified by
what A's production code names through `B::` (test files and `#[cfg(test)]` modules excluded)
and how B defines those items. The classification is textual, so it is a starting map rather
than a proof; the layering tool remains the authority on membership.

| Class | Edges |
|---|---|
| type-only | 46 |
| constant-only | 2 |
| type-and-constant | 1 |
| behavioural | 68 |
| needs-review | 9 |

Type- and constant-only edges move with the plain model types (slice 5.3). Behavioural
edges need a port or a move. Most of them are small pure helpers that live in a
higher-level module than their callers, so the cheapest cut is moving the helper down
into a leaf module inside the same crate. That changes no `Cargo.toml` and no worker
generation input, so it can land before the crate split.

## Behavioural edges by target

| Target | Callers | Items called |
|---|---|---|
| `admin` | client_auth, client_auth_client, core, oidc, passkey, provider_service, store | `hex_sha256`, `is_admin_setting_key` |
| `managed_calls` | agent_api, artifacts, cli, core, persistence | `call`, `compact_metadata`, `exchange`, `generated_update`, `review`, `start`, `wiring` |
| `product_auth` | admin, local_operator, oidc, passkey, store | `cookie_value`, `hash_password`, `new_session_token`, `new_user_id`, `normalize_username`, `verify_password`, `verify_unknown_user_password` |
| `auth_plugins` | core_security, oidc, plugin_activation, store | `decode_private_json`, `disabled`, `load` |
| `core_passkeys` | plugin_activation, plugin_catalog, plugin_runtime, store | `ceremony`, `count_admin`, `count_user`, `delete_admin`, `delete_user`, `insert_admin`, `insert_user`, `list_admin` … |
| `client_auth` | client_auth_client, oidc, store | `code_challenge`, `decode_public_key`, `device_fingerprint`, `new_code_verifier`, `new_signing_key`, `public_key_to_base64`, `signed_exchange_request`, `signed_proof_headers` … |
| `client_auth_client` | acp_bridge, cli, product_cli | `logged_in_origins`, `new`, `normalized_base_url` |
| `core` | session_context, session_folders, store | `bound_history_page`, `bound_restored_hot_log`, `default`, `is_authored_cmid`, `is_persisted_setting_key`, `merge_session_order`, `question_summary_title` |
| `core_security` | plugin_activation, plugin_storage, store | `inspect`, `load`, `namespace_path` |
| `session_folders` | core, product_cli, store | `local`, `parse`, `parse_stored`, `stored_json` |
| `artifacts` | managed_calls, store | `collect_references`, `new` |
| `passkey` | auth_plugins, store | `valid_reauth_interval` |
| `persistence` | core, store | `compact_canonical_tool_output`, `compact_inbound_event`, `persisted_event_payload`, `take_persisted_cmid` |
| `plugin_catalog` | plugin_runtime, provider_catalog | `desired_plugin_compatibility`, `host_releases` |
| `plugin_storage` | core_passkeys, store | `postgres`, `set_postgres_search_path`, `sqlite_files` |
| `usage` | local_operator, provider_info | `intern_usage_str`, `now_ms` |
| `acp_bridge` | cli | `serve` |
| `agent_api` | cli | `serve` |
| `composition` | cli | `check_file` |
| `controller_hooks` | cli | `init_tracing`, `serve` |
| `local_operator` | cli | `run` |
| `machine_control` | usage | `default` |
| `observability` | otlp | `redact_message`, `sanitize_attributes` |
| `oidc` | plugin_activation | `load` |
| `otlp` | observability | `runtime_span` |
| `plugin_runtime` | plugin_catalog | `activate_selected`, `empty` |
| `product_cli` | cli | `run`, `run_api`, `run_dir`, `run_send`, `run_session` |
| `provider_info` | usage | `error`, `overlay_session_usage`, `unavailable` |
| `scheduler` | core | `fire_at_from_delay` |
| `session_context` | core | `allows_dispatch`, `read`, `without_preamble` |
| `store` | cli | `connect`, `postgres_to_sqlite` |
| `telemetry_binding` | store | `apply`, `intent` |

## Ports

| Group | Port |
|---|---|
| `admin::hex_sha256` (7 callers) | Move to a leaf digest helper; `admin` keeps its admin logic only. |
| `product_auth` token, password and username helpers (5 callers) | Move the pure helpers (`new_session_token`, `new_user_id`, `hash_password`, `verify_password`, `normalize_username`, `cookie_value`) to a leaf credential module that depends on no store or Hub type. |
| `auth_plugins::decode_private_json`, `core_passkeys::validate_legacy_host`, `client_auth::decode_public_key` | Leaf codec helpers beside the types they decode. |
| `usage::now_ms` | A leaf clock helper. |
| `store` → `core_passkeys` / `core_security` / `persistence` / `telemetry_binding` | The store calls back into owners of policy. These become traits the owners implement (or plain data passed in), defined beside the store row types in 5.3, so the store never names `core` or the Hub. |
| `core` → `session_context`, `scheduler`, `managed_calls`, `persistence` | Hub-side behaviour; stays inside `cowboy-hub` (5.5), which these modules join. |
| `cli` → `acp_bridge`, `agent_api`, `product_cli`, `store`, … | Entry points; move to the binaries' crates with the owner-specific modules (5.6). |
| `otlp` ↔ `observability` | One telemetry leaf: `redact_message` / `sanitize_attributes` move beside the span types. |

## Needs review (9 edges)

Edges whose referenced names did not resolve textually (mostly enum variants such as
`Signal::Logs` and re-exported items):

- `composition` → `otlp`: `Signal`
- `core_passkeys` → `store`: `NamespacePrepared`
- `plugin_catalog` → `otlp`: `Logs`, `Metrics`, `Signal`, `Traces`
- `product_cli` → `managed_calls`: `valid_id`
- `session_context` → `managed_calls`: `valid_id`
- `store` → `managed_calls`: `Continue`, `Fresh`, `Record`, `Request`, `State`, `canonical_digest`
- `telemetry_plugin` → `observability`: `default`
- `telemetry_plugin` → `otlp`: `Export`, `Logs`, `MAX_ITEMS`, `Metrics`, `Signal`, `Traces`
- `telemetry_plugin` → `telemetry_binding`: `BindingDigest`, `BindingSnapshot`, `valid_service`
