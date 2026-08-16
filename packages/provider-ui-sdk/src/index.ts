/**
 * Cowboy Provider UI SDK v2.
 *
 * This package is intentionally renderer-agnostic. Providers ship a closed,
 * data-only UI and logic IR; Cowboy supplies the renderer and privileged
 * effects. No Provider JavaScript is evaluated in the browser.
 */

export const PROVIDER_PACKAGE_SCHEMA_VERSION = 2 as const;
export const PROVIDER_RELEASE_SCHEMA_VERSION = 2 as const;
export const PROVIDER_UI_SCHEMA_MIN_VERSION = 1 as const;
export const PROVIDER_UI_SCHEMA_VERSION = 2 as const;
export const PROVIDER_HOST_SCHEMA_MIN_VERSION = 1 as const;
export const PROVIDER_HOST_SCHEMA_VERSION = 2 as const;
export const PROVIDER_MACHINE_CONTRACT_VERSION = 4 as const;
export const PROVIDER_SDK_VERSION = "2.4.0" as const;

export type SurfaceSlot =
  | "card"
  | "setup"
  | "settings"
  | "information"
  | "empty"
  | "loading"
  | "error"
  | "session";

export type Tone = "neutral" | "primary" | "success" | "warning" | "error";
export type LiteralValue = string | boolean | number;
export type ValueType = "string" | "bool" | "integer";

export type TextMatchExpression =
  | { op: "contains"; value: string }
  | { op: "all"; values: TextMatchExpression[] }
  | { op: "any"; values: TextMatchExpression[] };

export type TextValue =
  | { source: "literal"; value: string }
  | { source: "state"; field: string }
  | {
    source: "host";
    field:
      | "provider_version"
      | "installation_state"
      | "authentication_state"
      | "distribution_state"
      | "machine_name"
      | "error_detail";
  };

export type BoolExpression =
  | { op: "state_equals"; field: string; value: LiteralValue }
  | {
    op: "host_equals";
    field:
      | "installed"
      | "auth_ready"
      | "auth_required"
      | "machine_online"
      | "upgrade_available";
    value: boolean;
  }
  | { op: "all"; values: BoolExpression[] }
  | { op: "any"; values: BoolExpression[] }
  | { op: "not"; value: BoolExpression };

export interface MessageEmission {
  message: string;
  payload: Record<string, LiteralValue>;
}

export type UiNode =
  | {
    component: "stack";
    direction: "row" | "column" | "responsive";
    gap: "xs" | "sm" | "md" | "lg";
    wrap?: boolean;
    children: UiNode[];
    visible_when?: BoolExpression;
  }
  | {
    component: "text";
    variant: "title" | "body" | "caption" | "code";
    value: TextValue;
    tone?: Tone;
  }
  | { component: "asset"; asset: string; size: "sm" | "md" | "lg" | "fill" }
  | { component: "badge"; label: TextValue; tone: Tone }
  | { component: "progress"; label: TextValue }
  | {
    component: "activity";
    indicator: ActivityIndicator;
    label: ActivityLabel;
    accessible_label: string;
  }
  | { component: "alert"; tone: Tone; title: TextValue; body: TextValue }
  | { component: "divider" }
  | {
    component: "button";
    label: TextValue;
    style: "primary" | "secondary" | "destructive";
    emit: MessageEmission;
    enabled_when?: BoolExpression;
  };

export type ActivityIndicator =
  | { kind: "progress_ring" }
  | { kind: "glyph_cycle"; frames: string[]; interval_ms: number }
  | { kind: "terminal_prompt"; interval_ms: number }
  | { kind: "asset_signal"; asset: string; interval_ms: number }
  | { kind: "asset_pulse"; asset: string; interval_ms: number };

export type ActivityTextEffect = "none" | "fade" | "shimmer";

export type ActivityLabel =
  | { kind: "text"; value: TextValue; effect: ActivityTextEffect }
  | {
    kind: "phrase_cycle";
    phrases: string[];
    interval_ms: number;
    suffix: string;
    effect: ActivityTextEffect;
  };

export interface UiAsset {
  id: string;
  role: "logo" | "icon" | "loading" | "illustration";
  media_type: string;
  digest: string;
  accessible_label: string;
  content:
    | {
      kind: "vector_path";
      view_box: string;
      path: string;
      fill?: string;
      gradient?: VectorGradient;
    }
    | { kind: "inline"; base64: string };
}

export interface VectorGradient {
  x1_percent: number;
  y1_percent: number;
  x2_percent: number;
  y2_percent: number;
  stops: Array<{ offset_percent: number; color: string }>;
}

export type ThoughtVariant =
  | "timeline"
  | "workcell"
  | "signal"
  | "terminal";
export type ThoughtDensity = "compact" | "comfortable";
export type CurrentThoughtSurface = "plain" | "soft";
export type ProviderAuthenticationPresentation = "account" | "api_key";

export interface TranscriptPresentationContract {
  schema_version: 1;
  thought: {
    variant: ThoughtVariant;
    density: ThoughtDensity;
    active_label?: string;
    current_surface: CurrentThoughtSurface;
  };
}

interface HostIntegrationBase {
  conversation_compaction?: {
    aliases: string[];
    fallback_command: string;
  };
  account_usage?: {
    provider: "openai" | "anthropic" | "deepseek" | "gemini" | "xai";
  };
  features: Array<"cache_protection_v1">;
  tool_presentations: Array<{
    tool_name: string;
    renderer: "todo_list_v1";
  }>;
}

export type HostIntegrationContract =
  & HostIntegrationBase
  & (
    | { schema_version: 1; transcript?: never }
    | { schema_version: 2; transcript: TranscriptPresentationContract }
  );

export interface StateField {
  id: string;
  value_type: ValueType;
  initial: LiteralValue;
}

export interface MessageSchema {
  id: string;
  payload: Record<string, ValueType>;
}

export interface ReducerRule {
  message: string;
  assignments: Array<{ field: string; value: AssignmentValue }>;
  effect?: string;
}

export type AssignmentValue =
  | { source: "literal"; value: LiteralValue }
  | { source: "state"; field: string }
  | { source: "message"; field: string };

export type EffectCapability =
  | "begin_service_authentication"
  | "logout_service_authentication"
  | "install_on_machine"
  | "upgrade_on_machine"
  | "request_uninstall_plan"
  | "open_external_documentation";

export type PrivateComponentKind =
  | "provider_cli"
  | "provider_adapter"
  | "provider_gateway"
  | "acp_runtime";

export interface RuntimeComponentReference {
  kind: PrivateComponentKind;
  slot: string;
}

/** Closed, linked values resolved by Cowboy inside one exact Provider
 * generation. Strings are literals; object variants cannot execute code. */
export type RuntimeValue =
  | string
  | {
    source: "component_command";
    component: RuntimeComponentReference;
    prefix?: string;
    suffix?: string;
  }
  | {
    source: "sidecar_url";
    sidecar: string;
    prefix?: string;
    suffix?: string;
  };

export interface RuntimeSidecar {
  id: string;
  component: RuntimeComponentReference;
  arguments: string[];
  environment: Record<string, string>;
  auth_environment: string[];
  transport: {
    kind: "loopback_http_v1";
    listen_argument: string;
    health_path: string;
    timeout_ms: number;
  };
}

export interface EffectSchema {
  id: string;
  capability: EffectCapability;
  request: Record<string, ValueType>;
  success_message: string;
  failure_message: string;
}

export interface ProviderManifest {
  id: string;
  version: string;
  publisher: string;
  sdk_version: string;
  display: {
    name: string;
    vendor: string;
    summary: string;
    accent: string;
    secondary_accent: string;
    logo_asset: string;
    icon_asset: string;
  };
  ui: {
    schema_version: number;
    assets: UiAsset[];
    surfaces: Record<SurfaceSlot, UiNode>;
  };
  logic: {
    schema_version: number;
    state: StateField[];
    messages: MessageSchema[];
    reducers: ReducerRule[];
    effects: EffectSchema[];
  };
  configuration: {
    schema_version: number;
    presets: Array<{
      id: string;
      name: string;
      detail: string;
      is_default: boolean;
      values: Record<string, string>;
    }>;
    options: Array<{
      id: string;
      order: number;
      layout: "standard" | "full_width";
      availability: "live_session" | "idle_or_stopped";
    }>;
  };
  host: HostIntegrationContract;
  runtime: {
    driver_schema: number;
    protocol: string;
    entrypoint: string;
    behavior: {
      schema_version: number;
      permission:
        | "portable_v1"
        | "acp_config_full_access_v1"
        | "acp_session_mode_bypass_permissions_v1"
        | "acp_session_mode_yolo_v1"
        | "xai_session_v1";
      session:
        | "portable_v1"
        | "stable_preset_system_prompt_v1"
        | "xai_session_v1";
      turn_end: "portable_v1";
      configuration:
        | "portable_v1"
        | "acp_config_options_v1"
        | "xai_session_v1"
        | "anthropic_gateway_v1"
        | "openai_gateway_v1";
      default_preferences: Record<string, LiteralValue>;
      error_rules: Array<{
        when: TextMatchExpression;
        user_detail?: string;
        classification?: string;
        keep_worker_alive?: boolean;
        retry_once_without_visible_update?: boolean;
      }>;
    };
    arguments: RuntimeValue[];
    environment: Record<string, RuntimeValue>;
    sidecars: RuntimeSidecar[];
    remove_environment: string[];
    remove_environment_prefixes: string[];
    dependencies: Array<{
      id: string;
      version: string;
      source: string;
      integrity: string;
      private: boolean;
    }>;
    platforms: Array<{
      os: "linux" | "macos";
      architecture: "x86_64" | "aarch64";
      payload_digest: string;
      launch_command: string;
      private_components: Array<{
        kind: PrivateComponentKind;
        slot: string;
        dependency: string;
        command: string;
      }>;
    }>;
    required_capabilities: Array<"provider.runtime.v1" | "provider.gateway.v1">;
  };
  authentication: {
    schema_version: number;
    required: boolean;
    portable_schema: string;
    projection_schema: string;
    refresh: "service" | "compare_and_swap";
    methods: Array<{
      id: string;
      label: string;
      flow: "device_code" | "browser_code" | "secret_input" | "service_broker";
      executor:
        | {
          kind: "secret_input_v1";
          bundle_key: string;
          verification_url: string;
        }
        | {
          kind: "command_v1";
          component: {
            kind:
              | "provider_cli"
              | "provider_adapter"
              | "provider_gateway"
              | "acp_runtime";
            slot: string;
          };
          arguments: string[];
          terminal: "pipes" | "pty";
          challenge: "device_code" | "browser_code";
          environment: Record<string, string>;
          preflight: Array<
            | {
              kind: "json_string_set_v1";
              relative_path: string;
              path: string[];
              value: string;
            }
            | {
              kind: "env_file_key_required_v1";
              relative_path: string;
              key: string;
            }
          >;
        };
      required_bundle_keys: string[];
    }>;
    credential_files: Array<{
      bundle_key: string;
      relative_path: string;
      required: boolean;
    }>;
    environment_projection: Record<string, string>;
  };
  compatibility: {
    min_controller_contract: number;
    max_controller_contract: number;
    min_machine_contract: number;
    max_machine_contract: number;
    ui_component_fingerprint: string;
    auth_contract_fingerprint: string;
  };
}

/** Sanitized manifest projection delivered to ordinary Cowboy UI. Runtime
 * transports, dependency pins, and credential paths stay Provider-private. */
export type ProviderUiManifest =
  & Pick<
    ProviderManifest,
    | "id"
    | "version"
    | "publisher"
    | "sdk_version"
    | "display"
    | "ui"
    | "logic"
    | "configuration"
    | "host"
    | "compatibility"
  >
  & {
    authentication:
      & Pick<ProviderManifest["authentication"], "schema_version" | "required">
      & {
        presentation?: ProviderAuthenticationPresentation;
        methods: Array<
          Pick<
            ProviderManifest["authentication"]["methods"][number],
            "id" | "label" | "flow"
          >
        >;
      };
  };

export interface ProviderCatalogEntry {
  provider_id: string;
  provider_version: string;
  package_digest: string;
  artifact_digest: string | null;
  /** Public scope for one credential shared by compatible Providers. */
  authentication_scope: string;
  release_state: "unbound" | "ready";
  release_detail?: string;
  publisher: string;
  contract_fingerprint: string;
  supported_platforms: Array<{
    os: "linux" | "macos";
    architecture: "x86_64" | "aarch64";
  }>;
  manifest: ProviderUiManifest;
}

export interface ProviderContractInventory {
  provider_sdk_version: string;
  min_package_schema: number;
  max_package_schema: number;
  min_release_schema: number;
  max_release_schema: number;
  min_ui_schema: number;
  max_ui_schema: number;
  min_host_schema: number;
  max_host_schema: number;
  machine_contract: number;
}

export type ProviderCompatibilityCode =
  | "capability_inventory_unavailable"
  | "capability_inventory_invalid"
  | "provider_sdk_unsupported"
  | "package_schema_unsupported"
  | "release_schema_unsupported"
  | "ui_schema_unsupported"
  | "host_schema_unsupported"
  | "machine_contract_unsupported"
  | "platform_unsupported";

export interface ProviderCompatibilityProblem {
  code: ProviderCompatibilityCode;
  detail: string;
}

export interface ProviderCompatibilityTarget {
  platform: "linux" | "macos";
  architecture: "x86_64" | "aarch64";
  provider_contracts?: ProviderContractInventory;
}

export interface ProviderAuthenticationStatus {
  provider_id: string;
  auth_generation: number;
  authentication_state:
    | "signed_out"
    | "authenticating"
    | "ready"
    | "expired"
    | "error";
  distribution_state:
    | "none"
    | "pending"
    | "current"
    | "partial"
    | "failed"
    | "revoking";
  auth_contract_fingerprint: string;
  /** Matches ProviderCatalogEntry.authentication_scope. */
  authentication_scope: string;
  portable_schema: string;
  projection_schema: string;
  account_label?: string;
  updated_at_ms: number;
}

export function resolveProviderAuthenticationPresentation(
  authentication: ProviderUiManifest["authentication"],
): ProviderAuthenticationPresentation {
  if (authentication.presentation !== undefined) {
    return authentication.presentation;
  }
  return authentication.required && authentication.methods.length > 0 &&
      authentication.methods.every((method) => method.flow === "secret_input")
    ? "api_key"
    : "account";
}

export interface ProviderCatalogResponse {
  providers: ProviderCatalogEntry[];
  authentications: ProviderAuthenticationStatus[];
  authentication_executors: ProviderAuthenticationExecutor[];
}

/** Exact active Provider generations on connected Machines that may execute a
 * temporary Cowboy Service authentication flow. Machine identity stays
 * private because the Service UI only needs a trusted release identity. */
export interface ProviderAuthenticationExecutor {
  provider_id: string;
  provider_version: string;
  generation_digest: string;
}

export interface MachineProviderInventory {
  provider_id: string;
  provider_version: string;
  generation_digest: string;
  contract_fingerprint: string;
  state:
    | "missing"
    | "installing"
    | "active"
    | "uninstalling"
    | "incompatible"
    | "failed";
  rollback_generation_digest?: string;
  active_session_leases: number;
  auth_generation?: number;
  replica_state:
    | "absent"
    | "pending"
    | "storing"
    | "current"
    | "failed"
    | "revoking";
  materialization_state: "not_installed" | "applying" | "current" | "failed";
  detail?: string;
}

export interface ProviderHostContext {
  provider_version: string;
  installation_state: string;
  authentication_state: string;
  distribution_state: string;
  machine_name: string;
  error_detail: string;
  installed: boolean;
  auth_ready: boolean;
  auth_required: boolean;
  machine_online: boolean;
  upgrade_available: boolean;
}

export type ProviderState = Record<string, LiteralValue>;

export interface ProviderTransition {
  state: ProviderState;
  effect?: EffectSchema;
}

export function defineProviderManifest<const T extends ProviderManifest>(
  manifest: T,
): T {
  validateProviderManifest(manifest);
  return manifest;
}

export function initialProviderState(
  manifest: ProviderUiManifest,
): ProviderState {
  return Object.fromEntries(
    manifest.logic.state.map((field) => [field.id, field.initial]),
  );
}

export function transitionProvider(
  manifest: ProviderUiManifest,
  state: ProviderState,
  emission: MessageEmission,
): ProviderTransition {
  const schema = manifest.logic.messages.find((message) =>
    message.id === emission.message
  );
  if (!schema) {
    throw new Error(`Provider emitted unknown message ${emission.message}`);
  }
  validatePayload(schema, emission.payload);
  const next = { ...state };
  let effect: EffectSchema | undefined;
  for (
    const reducer of manifest.logic.reducers.filter((rule) =>
      rule.message === emission.message
    )
  ) {
    for (const assignment of reducer.assignments) {
      switch (assignment.value.source) {
        case "literal":
          next[assignment.field] = assignment.value.value;
          break;
        case "state":
          next[assignment.field] = state[assignment.value.field] ?? "";
          break;
        case "message":
          next[assignment.field] = emission.payload[assignment.value.field] ??
            "";
          break;
        default:
          assertNever(assignment.value);
      }
    }
    if (reducer.effect) {
      const resolved = manifest.logic.effects.find((candidate) =>
        candidate.id === reducer.effect
      );
      if (!resolved) {
        throw new Error(
          `Provider reducer references unknown effect ${reducer.effect}`,
        );
      }
      effect = resolved;
    }
  }
  return effect ? { state: next, effect } : { state: next };
}

export function resolveText(
  value: TextValue,
  state: ProviderState,
  host: ProviderHostContext,
): string {
  switch (value.source) {
    case "literal":
      return value.value;
    case "state":
      return String(state[value.field] ?? "");
    case "host":
      return host[value.field];
    default:
      return assertNever(value);
  }
}

export function evaluateExpression(
  expression: BoolExpression | undefined,
  state: ProviderState,
  host: ProviderHostContext,
): boolean {
  if (!expression) return true;
  switch (expression.op) {
    case "state_equals":
      return state[expression.field] === expression.value;
    case "host_equals":
      return host[expression.field] === expression.value;
    case "all":
      return expression.values.every((value) =>
        evaluateExpression(value, state, host)
      );
    case "any":
      return expression.values.some((value) =>
        evaluateExpression(value, state, host)
      );
    case "not":
      return !evaluateExpression(expression.value, state, host);
    default:
      return assertNever(expression);
  }
}

export function validateProviderCatalog(
  input: unknown,
): ProviderCatalogResponse {
  if (
    !isRecord(input) || !Array.isArray(input.providers) ||
    !Array.isArray(input.authentications) ||
    !Array.isArray(input.authentication_executors)
  ) {
    throw new Error("Invalid Provider Catalog response");
  }
  for (const raw of input.providers) {
    if (
      !isRecord(raw) || typeof raw.provider_id !== "string" ||
      !isIdentifier(raw.provider_id) ||
      typeof raw.provider_version !== "string" ||
      !parseSemanticVersion(raw.provider_version) ||
      typeof raw.package_digest !== "string" || !isDigest(raw.package_digest) ||
      (raw.artifact_digest !== null &&
        (typeof raw.artifact_digest !== "string" ||
          !isDigest(raw.artifact_digest))) ||
      typeof raw.authentication_scope !== "string" ||
      !isIdentifier(raw.authentication_scope) ||
      (raw.release_state !== "unbound" && raw.release_state !== "ready") ||
      (raw.release_detail !== undefined &&
        (typeof raw.release_detail !== "string" ||
          raw.release_detail.length > 2_048)) ||
      typeof raw.publisher !== "string" || !isIdentifier(raw.publisher) ||
      typeof raw.contract_fingerprint !== "string" ||
      !isDigest(raw.contract_fingerprint) ||
      !Array.isArray(raw.supported_platforms) ||
      !raw.supported_platforms.every((target) =>
        isRecord(target) && ["linux", "macos"].includes(String(target.os)) &&
        ["x86_64", "aarch64"].includes(String(target.architecture))
      ) || !isRecord(raw.manifest)
    ) {
      throw new Error("Invalid Provider Catalog entry");
    }
    const manifest = raw.manifest;
    const authentication = manifest.authentication;
    if (
      "runtime" in manifest ||
      !isRecord(authentication) ||
      [
        "portable_schema",
        "projection_schema",
        "refresh",
        "credential_files",
        "environment_projection",
      ].some((key) => key in authentication) ||
      !Array.isArray(authentication.methods) ||
      authentication.methods.some((method) =>
        isRecord(method) &&
        ("executor" in method || "required_bundle_keys" in method)
      )
    ) {
      throw new Error("Provider Catalog exposed a private Provider contract");
    }
    validateProviderUiManifest(manifest);
    if (
      raw.provider_id !== manifest.id ||
      raw.provider_version !== manifest.version ||
      raw.publisher !== manifest.publisher
    ) {
      throw new Error("Provider Catalog identity mismatch");
    }
    if (
      !raw.package_digest || !raw.contract_fingerprint ||
      (raw.release_state === "ready" && !raw.artifact_digest) ||
      (raw.release_state === "unbound" && raw.artifact_digest !== null)
    ) {
      throw new Error("Provider Catalog entry is not content addressed");
    }
  }
  for (const raw of input.authentications) {
    if (
      !isRecord(raw) || typeof raw.provider_id !== "string" ||
      !isIdentifier(raw.provider_id) ||
      typeof raw.auth_generation !== "number" ||
      !Number.isSafeInteger(raw.auth_generation) ||
      raw.auth_generation < 0 ||
      !["signed_out", "authenticating", "ready", "expired", "error"].includes(
        String(raw.authentication_state),
      ) || (raw.auth_generation === 0 &&
        raw.authentication_state !== "authenticating" &&
        raw.authentication_state !== "error") ||
      !["none", "pending", "current", "partial", "failed", "revoking"].includes(
        String(raw.distribution_state),
      ) || typeof raw.auth_contract_fingerprint !== "string" ||
      !isDigest(raw.auth_contract_fingerprint) ||
      typeof raw.authentication_scope !== "string" ||
      !isIdentifier(raw.authentication_scope) ||
      typeof raw.portable_schema !== "string" ||
      !isIdentifier(raw.portable_schema) ||
      typeof raw.projection_schema !== "string" ||
      !isIdentifier(raw.projection_schema) ||
      (raw.account_label !== undefined &&
        typeof raw.account_label !== "string") ||
      typeof raw.updated_at_ms !== "number" ||
      !Number.isSafeInteger(raw.updated_at_ms)
    ) {
      throw new Error("Invalid Provider authentication status");
    }
  }
  const executorIdentities = new Set<string>();
  for (const raw of input.authentication_executors) {
    if (
      !isRecord(raw) ||
      !hasOnlyKeys(raw, [
        "provider_id",
        "provider_version",
        "generation_digest",
      ]) ||
      typeof raw.provider_id !== "string" ||
      !isIdentifier(raw.provider_id) ||
      typeof raw.provider_version !== "string" ||
      !parseSemanticVersion(raw.provider_version) ||
      typeof raw.generation_digest !== "string" ||
      !isDigest(raw.generation_digest)
    ) {
      throw new Error("Invalid Provider authentication executor");
    }
    const identity = `${raw.provider_id}:${raw.provider_version}:${raw.generation_digest}`;
    if (executorIdentities.has(identity)) {
      throw new Error("Duplicate Provider authentication executor");
    }
    executorIdentities.add(identity);
  }
  return input as unknown as ProviderCatalogResponse;
}

export function validateMachineProviderInventory(
  input: unknown,
): MachineProviderInventory[] {
  if (!Array.isArray(input)) {
    throw new Error("Invalid Machine Provider inventory");
  }
  const identities = new Set<string>();
  for (const raw of input) {
    if (
      !isRecord(raw) || typeof raw.provider_id !== "string" ||
      !isIdentifier(raw.provider_id) ||
      typeof raw.provider_version !== "string" ||
      !parseSemanticVersion(raw.provider_version) ||
      typeof raw.generation_digest !== "string" ||
      !isDigest(raw.generation_digest) ||
      typeof raw.contract_fingerprint !== "string" ||
      !isDigest(raw.contract_fingerprint) ||
      ![
        "missing",
        "installing",
        "active",
        "uninstalling",
        "incompatible",
        "failed",
      ].includes(
        String(raw.state),
      ) || (raw.rollback_generation_digest !== undefined &&
        (typeof raw.rollback_generation_digest !== "string" ||
          !isDigest(raw.rollback_generation_digest))) ||
      typeof raw.active_session_leases !== "number" ||
      !Number.isSafeInteger(raw.active_session_leases) ||
      raw.active_session_leases < 0 ||
      (raw.auth_generation !== undefined &&
        (typeof raw.auth_generation !== "number" ||
          !Number.isSafeInteger(raw.auth_generation) ||
          raw.auth_generation < 1)) ||
      !["absent", "pending", "storing", "current", "failed", "revoking"]
        .includes(
          String(raw.replica_state),
        ) ||
      !["not_installed", "applying", "current", "failed"].includes(
        String(raw.materialization_state),
      ) || (raw.detail !== undefined &&
        (typeof raw.detail !== "string" || raw.detail.length > 2_048))
    ) {
      throw new Error("Invalid Machine Provider inventory entry");
    }
    if (identities.has(raw.provider_id)) {
      throw new Error("Duplicate Machine Provider inventory entry");
    }
    identities.add(raw.provider_id);
  }
  return input as MachineProviderInventory[];
}

export function validateProviderManifest(
  input: unknown,
): asserts input is ProviderManifest {
  validateProviderUiManifest(input);
  const full = input as unknown as Record<string, unknown>;
  const runtime = full.runtime;
  const authentication = full.authentication;
  const compatibility = full.compatibility;
  if (
    !isRecord(runtime) || runtime.driver_schema !== 1 ||
    typeof runtime.protocol !== "string" ||
    !runtime.protocol || typeof runtime.entrypoint !== "string" ||
    !isIdentifier(runtime.entrypoint) ||
    !isRecord(runtime.behavior) || !Array.isArray(runtime.arguments) ||
    !runtime.arguments.every(isRuntimeValue) ||
    !isRecord(runtime.environment) ||
    !Object.entries(runtime.environment).every(([name, value]) =>
      isEnvironmentName(name) && isRuntimeValue(value)
    ) ||
    !Array.isArray(runtime.sidecars) ||
    !Array.isArray(runtime.remove_environment) ||
    !runtime.remove_environment.every((value) =>
      typeof value === "string" && isEnvironmentName(value)
    ) ||
    !Array.isArray(runtime.remove_environment_prefixes) ||
    !Array.isArray(runtime.dependencies) ||
    !Array.isArray(runtime.platforms) || runtime.platforms.length === 0 ||
    !Array.isArray(runtime.required_capabilities) ||
    !isRecord(authentication) ||
    typeof authentication.portable_schema !== "string" ||
    typeof authentication.projection_schema !== "string" ||
    !Array.isArray(authentication.credential_files) ||
    !isRecord(authentication.environment_projection) || !isRecord(compatibility)
  ) {
    throw new Error(
      "Invalid complete Provider runtime or authentication contract",
    );
  }
  validateProviderBehavior(runtime.behavior);
  const dependencies = new Map<string, string>();
  for (const raw of runtime.dependencies) {
    if (
      !isRecord(raw) || typeof raw.id !== "string" || !isIdentifier(raw.id) ||
      typeof raw.version !== "string" || !isExactVersion(raw.version) ||
      typeof raw.source !== "string" || !raw.source ||
      typeof raw.integrity !== "string" ||
      !raw.integrity || typeof raw.private !== "boolean" ||
      dependencies.has(raw.id)
    ) {
      throw new Error("Invalid or duplicate exact Provider dependency");
    }
    dependencies.set(raw.id, raw.version);
  }
  const platforms = new Set<string>();
  const platformComponents: Set<string>[] = [];
  const gatewayComponents: Set<string>[] = [];
  for (const raw of runtime.platforms) {
    if (
      !isRecord(raw) || !["linux", "macos"].includes(String(raw.os)) ||
      !["x86_64", "aarch64"].includes(String(raw.architecture)) ||
      typeof raw.payload_digest !== "string" ||
      typeof raw.launch_command !== "string" ||
      !isIdentifier(raw.launch_command) ||
      !Array.isArray(raw.private_components) ||
      raw.private_components.length === 0
    ) {
      throw new Error("Invalid Provider platform payload");
    }
    const target = `${raw.os}/${raw.architecture}`;
    if (platforms.has(target)) {
      throw new Error("Duplicate Provider platform payload");
    }
    platforms.add(target);
    const slots = new Set<string>();
    const commands = new Set<string>();
    const components = new Set<string>();
    const gateways = new Set<string>();
    for (const component of raw.private_components) {
      if (
        !isRecord(component) ||
        !["provider_cli", "provider_adapter", "provider_gateway", "acp_runtime"]
          .includes(
            String(component.kind),
          ) ||
        typeof component.slot !== "string" || !isIdentifier(component.slot) ||
        typeof component.dependency !== "string" ||
        !dependencies.has(component.dependency) ||
        typeof component.command !== "string" ||
        !isIdentifier(component.command)
      ) {
        throw new Error("Invalid Provider private component binding");
      }
      const slot = `${component.kind}/${component.slot}`;
      if (slots.has(slot) || commands.has(component.command)) {
        throw new Error("Duplicate Provider private component binding");
      }
      slots.add(slot);
      commands.add(component.command);
      components.add(slot);
      if (component.kind === "provider_gateway") gateways.add(slot);
    }
    if (!commands.has(raw.launch_command)) {
      throw new Error(
        "Provider launch command is not exported by its platform payload",
      );
    }
    platformComponents.push(components);
    gatewayComponents.push(gateways);
  }
  const sidecarIds = new Set<string>();
  const sidecarComponents = new Set<string>();
  const sidecarAuthEnvironment = new Set<string>();
  for (const raw of runtime.sidecars) {
    if (
      !isRecord(raw) || typeof raw.id !== "string" ||
      !isIdentifier(raw.id) || sidecarIds.has(raw.id) ||
      !isRecord(raw.component) || raw.component.kind !== "provider_gateway" ||
      typeof raw.component.slot !== "string" ||
      !isIdentifier(raw.component.slot) || !Array.isArray(raw.arguments) ||
      raw.arguments.length > 64 ||
      !raw.arguments.every((value) =>
        typeof value === "string" && value.length <= 4_096 &&
        !value.includes("\0")
      ) || !isRecord(raw.environment) ||
      !Object.entries(raw.environment).every(([name, value]) =>
        isEnvironmentName(name) && typeof value === "string" &&
        value.length <= 32_768 && !value.includes("\0")
      ) || !Array.isArray(raw.auth_environment) ||
      !raw.auth_environment.every((name) =>
        typeof name === "string" && isEnvironmentName(name)
      ) || !isRecord(raw.transport) ||
      raw.transport.kind !== "loopback_http_v1" ||
      typeof raw.transport.listen_argument !== "string" ||
      !/^--[a-z0-9-]{1,62}$/.test(raw.transport.listen_argument) ||
      typeof raw.transport.health_path !== "string" ||
      !isSafeHealthPath(raw.transport.health_path) ||
      typeof raw.transport.timeout_ms !== "number" ||
      !Number.isSafeInteger(raw.transport.timeout_ms) ||
      raw.transport.timeout_ms < 100 || raw.transport.timeout_ms > 120_000
    ) {
      throw new Error("Invalid Provider runtime sidecar");
    }
    const component = `${raw.component.kind}/${raw.component.slot}`;
    if (
      sidecarComponents.has(component) ||
      !platformComponents.every((available) => available.has(component))
    ) {
      throw new Error("Invalid Provider runtime sidecar component");
    }
    for (const name of raw.auth_environment) {
      if (sidecarAuthEnvironment.has(name)) {
        throw new Error(
          "Provider auth environment is forwarded to multiple sidecars",
        );
      }
      sidecarAuthEnvironment.add(name);
    }
    sidecarIds.add(raw.id);
    sidecarComponents.add(component);
  }
  for (
    const value of [
      ...(runtime.arguments as RuntimeValue[]),
      ...(Object.values(runtime.environment) as RuntimeValue[]),
    ]
  ) {
    if (typeof value === "string") continue;
    if (value.source === "sidecar_url") {
      if (!sidecarIds.has(value.sidecar)) {
        throw new Error("Provider runtime value references an unknown sidecar");
      }
      continue;
    }
    const component = `${value.component.kind}/${value.component.slot}`;
    if (!platformComponents.every((available) => available.has(component))) {
      throw new Error(
        "Provider runtime value component is unavailable on a platform",
      );
    }
  }
  if (
    gatewayComponents.some((components) =>
      [...components].some((component) => !sidecarComponents.has(component))
    )
  ) {
    throw new Error("Provider gateway component has no runtime sidecar");
  }
  const capabilities = new Set(runtime.required_capabilities);
  const usesGateway = ["anthropic_gateway_v1", "openai_gateway_v1"].includes(
    String(runtime.behavior.configuration),
  );
  const declaresGateway = runtime.sidecars.length > 0;
  if (
    !runtime.required_capabilities.every((value) =>
      value === "provider.runtime.v1" || value === "provider.gateway.v1"
    ) ||
    !capabilities.has("provider.runtime.v1") ||
    usesGateway !== declaresGateway ||
    capabilities.has("provider.gateway.v1") !== declaresGateway
  ) {
    throw new Error("Unknown or missing Provider runtime capability");
  }
  validateAuthentication(
    authentication,
    runtime.platforms,
    sidecarAuthEnvironment,
  );
  if (
    typeof compatibility.min_controller_contract !== "number" ||
    typeof compatibility.max_controller_contract !== "number" ||
    compatibility.min_controller_contract > 2 ||
    compatibility.max_controller_contract < 2 ||
    typeof compatibility.min_machine_contract !== "number" ||
    typeof compatibility.max_machine_contract !== "number" ||
    compatibility.min_machine_contract > 4 ||
    compatibility.max_machine_contract < 4 ||
    typeof compatibility.ui_component_fingerprint !== "string" ||
    typeof compatibility.auth_contract_fingerprint !== "string"
  ) {
    throw new Error("Provider is incompatible with this Cowboy contract");
  }
}

function validateProviderBehavior(input: Record<string, unknown>): void {
  const permissions = [
    "portable_v1",
    "acp_config_full_access_v1",
    "acp_session_mode_bypass_permissions_v1",
    "acp_session_mode_yolo_v1",
    "xai_session_v1",
  ];
  const sessions = [
    "portable_v1",
    "stable_preset_system_prompt_v1",
    "xai_session_v1",
  ];
  const configurations = [
    "portable_v1",
    "acp_config_options_v1",
    "xai_session_v1",
    "anthropic_gateway_v1",
    "openai_gateway_v1",
  ];
  if (
    input.schema_version !== 1 ||
    !permissions.includes(String(input.permission)) ||
    !sessions.includes(String(input.session)) ||
    input.turn_end !== "portable_v1" ||
    !configurations.includes(String(input.configuration)) ||
    !isRecord(input.default_preferences) ||
    !Object.values(input.default_preferences).every((value) =>
      literalType(value) !== undefined
    ) ||
    !Array.isArray(input.error_rules)
  ) {
    throw new Error("Invalid Provider behavior contract");
  }
  for (const rule of input.error_rules) {
    if (!isRecord(rule) || !isRecord(rule.when)) {
      throw new Error("Invalid Provider error rule");
    }
    validateTextMatch(rule.when, 0);
    if (
      rule.user_detail !== undefined && typeof rule.user_detail !== "string"
    ) {
      throw new Error("Invalid Provider error presentation");
    }
    if (
      rule.classification !== undefined &&
      (typeof rule.classification !== "string" ||
        !isIdentifier(rule.classification))
    ) {
      throw new Error("Invalid Provider error classification");
    }
    if (
      rule.retry_once_without_visible_update === true &&
      rule.keep_worker_alive !== true
    ) {
      throw new Error("Retryable Provider errors must keep the worker alive");
    }
  }
}

function validateTextMatch(
  input: Record<string, unknown>,
  depth: number,
): void {
  if (depth > 8) throw new Error("Provider error matcher is too deeply nested");
  if (
    input.op === "contains" && typeof input.value === "string" &&
    input.value.length > 0
  ) return;
  if (
    (input.op === "all" || input.op === "any") && Array.isArray(input.values) &&
    input.values.length > 0 && input.values.length <= 16
  ) {
    for (const value of input.values) {
      if (!isRecord(value)) throw new Error("Invalid Provider error matcher");
      validateTextMatch(value, depth + 1);
    }
    return;
  }
  throw new Error("Invalid Provider error matcher");
}

function validateAuthentication(
  authentication: Record<string, unknown>,
  platforms: unknown[],
  sidecarAuthEnvironment: Set<string>,
): void {
  const environmentProjection = authentication.environment_projection;
  if (
    authentication.schema_version !== 1 ||
    typeof authentication.required !== "boolean" ||
    !["service", "compare_and_swap"].includes(String(authentication.refresh)) ||
    !Array.isArray(authentication.methods) ||
    !Array.isArray(authentication.credential_files) ||
    (authentication.required && authentication.methods.length === 0)
  ) {
    throw new Error("Invalid Provider authentication contract");
  }
  if (
    !isRecord(environmentProjection) ||
    !Object.entries(environmentProjection).every(
      ([name, bundleKey]) =>
        isEnvironmentName(name) && typeof bundleKey === "string" &&
        isIdentifier(bundleKey),
    ) ||
    [...sidecarAuthEnvironment].some((name) => !(name in environmentProjection))
  ) {
    throw new Error("Invalid Provider authentication environment projection");
  }
  const fileKeys = new Set<string>();
  for (const file of authentication.credential_files) {
    if (
      !isRecord(file) || typeof file.bundle_key !== "string" ||
      !isIdentifier(file.bundle_key) ||
      typeof file.relative_path !== "string" ||
      !isSafeRelativePath(file.relative_path) ||
      typeof file.required !== "boolean" || fileKeys.has(file.bundle_key)
    ) {
      throw new Error("Invalid Provider credential file");
    }
    fileKeys.add(file.bundle_key);
  }
  const methods = new Set<string>();
  for (const method of authentication.methods) {
    if (
      !isRecord(method) || typeof method.id !== "string" ||
      !isIdentifier(method.id) ||
      methods.has(method.id) || typeof method.label !== "string" ||
      !method.label ||
      !["device_code", "browser_code", "secret_input", "service_broker"]
        .includes(String(method.flow)) ||
      !isRecord(method.executor) || !Array.isArray(method.required_bundle_keys)
    ) {
      throw new Error("Invalid Provider authentication method");
    }
    methods.add(method.id);
    if (method.executor.kind === "secret_input_v1") {
      if (
        method.flow !== "secret_input" ||
        typeof method.executor.bundle_key !== "string" ||
        !method.required_bundle_keys.includes(method.executor.bundle_key)
      ) {
        throw new Error("Invalid Provider secret authentication executor");
      }
      continue;
    }
    if (
      method.executor.kind !== "command_v1" || method.flow === "secret_input" ||
      !isRecord(method.executor.component) ||
      typeof method.executor.component.kind !== "string" ||
      typeof method.executor.component.slot !== "string" ||
      !method.required_bundle_keys.every((key) =>
        typeof key === "string" && fileKeys.has(key)
      )
    ) {
      throw new Error("Invalid Provider command authentication executor");
    }
    const component = method.executor.component;
    for (const platform of platforms) {
      if (
        !isRecord(platform) || !Array.isArray(platform.private_components) ||
        !platform.private_components.some((candidate) =>
          isRecord(candidate) &&
          candidate.kind === component.kind && candidate.slot === component.slot
        )
      ) {
        throw new Error(
          "Provider authentication executor is unavailable on a platform",
        );
      }
    }
  }
}

export function validateProviderUiManifest(
  input: unknown,
): asserts input is ProviderUiManifest {
  if (
    !isRecord(input) || typeof input.id !== "string" ||
    typeof input.version !== "string" ||
    !isIdentifier(input.id) || !parseSemanticVersion(input.version) ||
    typeof input.publisher !== "string" || !isIdentifier(input.publisher) ||
    typeof input.sdk_version !== "string" ||
    !parseSemanticVersion(input.sdk_version) ||
    !isRecord(input.ui) ||
    typeof input.ui.schema_version !== "number" ||
    !Number.isSafeInteger(input.ui.schema_version) ||
    input.ui.schema_version < PROVIDER_UI_SCHEMA_MIN_VERSION ||
    input.ui.schema_version > PROVIDER_UI_SCHEMA_VERSION ||
    !Array.isArray(input.ui.assets) || !isRecord(input.ui.surfaces) ||
    !isRecord(input.logic) || input.logic.schema_version !== 1 ||
    !Array.isArray(input.logic.state) || !Array.isArray(input.logic.messages) ||
    !Array.isArray(input.logic.reducers) ||
    !Array.isArray(input.logic.effects) ||
    !isRecord(input.configuration) ||
    input.configuration.schema_version !== 1 ||
    !Array.isArray(input.configuration.presets) ||
    !Array.isArray(input.configuration.options) ||
    !isRecord(input.host) ||
    (input.host.schema_version !== 1 && input.host.schema_version !== 2) ||
    !Array.isArray(input.host.features) ||
    !Array.isArray(input.host.tool_presentations)
  ) {
    throw new Error("Invalid Provider manifest envelope");
  }
  if (!isCompatibleProviderSdkVersion(input.sdk_version)) {
    throw new Error(
      `Provider SDK ${input.sdk_version} is incompatible with Cowboy Provider SDK ${PROVIDER_SDK_VERSION}`,
    );
  }
  if (
    !isRecord(input.display) || typeof input.display.name !== "string" ||
    !input.display.name.trim() ||
    typeof input.display.vendor !== "string" || !input.display.vendor.trim() ||
    typeof input.display.summary !== "string" ||
    !input.display.summary.trim() ||
    typeof input.display.accent !== "string" ||
    !isColor(input.display.accent) ||
    typeof input.display.secondary_accent !== "string" ||
    !isColor(input.display.secondary_accent) ||
    typeof input.display.logo_asset !== "string" ||
    !isIdentifier(input.display.logo_asset) ||
    typeof input.display.icon_asset !== "string" ||
    !isIdentifier(input.display.icon_asset)
  ) {
    throw new Error("Invalid Provider display contract");
  }
  if (
    !isRecord(input.authentication) ||
    input.authentication.schema_version !== 1 ||
    typeof input.authentication.required !== "boolean" ||
    !Array.isArray(input.authentication.methods) ||
    !hasOnlyKeys(input.authentication, [
      "schema_version",
      "required",
      "presentation",
      "methods",
      "portable_schema",
      "projection_schema",
      "refresh",
      "credential_files",
      "environment_projection",
    ]) ||
    (input.authentication.presentation !== undefined &&
      !["account", "api_key"].includes(
        String(input.authentication.presentation),
      ))
  ) {
    throw new Error("Invalid Provider UI authentication contract");
  }
  const authMethods = new Set<string>();
  for (const method of input.authentication.methods) {
    if (
      !isRecord(method) || typeof method.id !== "string" ||
      !isIdentifier(method.id) ||
      authMethods.has(method.id) || typeof method.label !== "string" ||
      !method.label.trim() ||
      !hasOnlyKeys(method, [
        "id",
        "label",
        "flow",
        "executor",
        "required_bundle_keys",
      ]) ||
      !["device_code", "browser_code", "secret_input", "service_broker"]
        .includes(String(method.flow))
    ) {
      throw new Error("Invalid Provider UI authentication method");
    }
    authMethods.add(method.id);
  }
  if (input.authentication.required && authMethods.size === 0) {
    throw new Error("Authenticated Provider has no UI authentication method");
  }
  const inferredAuthenticationPresentation =
    input.authentication.required && authMethods.size > 0 &&
      input.authentication.methods.every((method) =>
        isRecord(method) && method.flow === "secret_input"
      )
      ? "api_key"
      : "account";
  if (
    input.authentication.presentation !== undefined &&
    input.authentication.presentation !== inferredAuthenticationPresentation
  ) {
    throw new Error(
      "Provider authentication presentation does not match its typed methods",
    );
  }
  if (
    !isRecord(input.compatibility) ||
    !isCompatibleContract(
      input.compatibility.min_controller_contract,
      input.compatibility.max_controller_contract,
      2,
    ) ||
    !isCompatibleContract(
      input.compatibility.min_machine_contract,
      input.compatibility.max_machine_contract,
      4,
    ) ||
    typeof input.compatibility.ui_component_fingerprint !== "string" ||
    !isDigest(input.compatibility.ui_component_fingerprint) ||
    typeof input.compatibility.auth_contract_fingerprint !== "string" ||
    !isDigest(input.compatibility.auth_contract_fingerprint)
  ) {
    throw new Error("Provider is incompatible with this Cowboy UI contract");
  }
  const compaction = input.host.conversation_compaction;
  if (
    compaction !== undefined &&
    (!isRecord(compaction) ||
      !hasOnlyKeys(compaction, ["aliases", "fallback_command"]) ||
      !Array.isArray(compaction.aliases) ||
      compaction.aliases.length === 0 ||
      !compaction.aliases.every((alias) =>
        typeof alias === "string" && isIdentifier(alias)
      ) ||
      typeof compaction.fallback_command !== "string" ||
      !compaction.aliases.includes(compaction.fallback_command))
  ) {
    throw new Error("Invalid Provider conversation compaction contract");
  }
  const accountUsage = input.host.account_usage;
  if (
    accountUsage !== undefined && (!isRecord(accountUsage) ||
      !hasOnlyKeys(accountUsage, ["provider"]) ||
      !["openai", "anthropic", "deepseek", "gemini", "xai"].includes(
        String(accountUsage.provider),
      ))
  ) {
    throw new Error("Invalid Provider account usage contract");
  }
  const hostKeys = [
    "schema_version",
    "conversation_compaction",
    "account_usage",
    "features",
    "tool_presentations",
  ];
  if (input.host.schema_version === 1) {
    if (!hasOnlyKeys(input.host, hostKeys)) {
      throw new Error(
        "Host integration schema 1 cannot declare Transcript presentation",
      );
    }
  } else {
    if (
      !hasOnlyKeys(input.host, [...hostKeys, "transcript"]) ||
      !isRecord(input.host.transcript) ||
      !hasOnlyKeys(input.host.transcript, ["schema_version", "thought"]) ||
      input.host.transcript.schema_version !== 1 ||
      !isRecord(input.host.transcript.thought)
    ) {
      throw new Error("Invalid Provider Transcript presentation contract");
    }
    const thought = input.host.transcript.thought;
    if (
      !hasOnlyKeys(thought, [
        "variant",
        "density",
        "active_label",
        "current_surface",
      ]) ||
      !["timeline", "workcell", "signal", "terminal"].includes(
        String(thought.variant),
      ) ||
      !["compact", "comfortable"].includes(String(thought.density)) ||
      !["plain", "soft"].includes(String(thought.current_surface)) ||
      (thought.active_label !== undefined &&
        (typeof thought.active_label !== "string" ||
          !thought.active_label.trim() ||
          thought.active_label.trim() !== thought.active_label ||
          [...thought.active_label].length > 64 ||
          [...thought.active_label].some((character) => {
            const code = character.codePointAt(0) ?? 0;
            return code < 0x20 || (code >= 0x7f && code <= 0x9f);
          })))
    ) {
      throw new Error("Invalid Provider Transcript thought presentation");
    }
  }
  if (
    !input.host.features.every((feature) => feature === "cache_protection_v1")
  ) {
    throw new Error("Unknown Provider host feature");
  }
  if (input.host.tool_presentations.length > 64) {
    throw new Error("Too many Provider tool presentations");
  }
  const toolPresentationNames = new Set<string>();
  for (const raw of input.host.tool_presentations) {
    if (
      !isRecord(raw) || typeof raw.tool_name !== "string" ||
      !hasOnlyKeys(raw, ["tool_name", "renderer"]) ||
      !raw.tool_name.trim() || raw.tool_name.trim() !== raw.tool_name ||
      raw.tool_name.length > 128 ||
      ![...raw.tool_name].every((character) => {
        const code = character.codePointAt(0) ?? 0;
        return code >= 0x20 && code <= 0x7e;
      }) ||
      raw.renderer !== "todo_list_v1"
    ) {
      throw new Error("Invalid Provider tool presentation");
    }
    if (toolPresentationNames.has(raw.tool_name)) {
      throw new Error("Duplicate Provider tool presentation");
    }
    toolPresentationNames.add(raw.tool_name);
  }
  if (input.configuration.presets.length > 32) {
    throw new Error("Too many Provider presets");
  }
  const presetIds = new Set<string>();
  let defaultPresets = 0;
  for (const raw of input.configuration.presets) {
    if (
      !isRecord(raw) || typeof raw.id !== "string" || !isIdentifier(raw.id) ||
      typeof raw.name !== "string" || !raw.name.trim() ||
      raw.name.length > 128 ||
      typeof raw.detail !== "string" || raw.detail.length > 512 ||
      typeof raw.is_default !== "boolean" ||
      !isRecord(raw.values) || Object.keys(raw.values).length === 0 ||
      Object.keys(raw.values).length > 32 ||
      !Object.entries(raw.values).every(([key, value]) =>
        isIdentifier(key) && typeof value === "string" && value.length > 0 &&
        value.length <= 4_096
      )
    ) {
      throw new Error("Invalid Provider configuration preset");
    }
    if (presetIds.has(raw.id)) {
      throw new Error("Duplicate Provider configuration preset");
    }
    presetIds.add(raw.id);
    defaultPresets += raw.is_default ? 1 : 0;
  }
  if (defaultPresets > 1) {
    throw new Error("Multiple default Provider configuration presets");
  }
  if (input.configuration.options.length > 64) {
    throw new Error("Too many Provider configuration option presentations");
  }
  const optionPresentationIds = new Set<string>();
  for (const raw of input.configuration.options) {
    if (
      !isRecord(raw) || typeof raw.id !== "string" || !isIdentifier(raw.id) ||
      !Number.isInteger(raw.order) || Number(raw.order) < 0 ||
      Number(raw.order) > 65_535 ||
      (raw.layout !== "standard" && raw.layout !== "full_width") ||
      (raw.availability !== "live_session" &&
        raw.availability !== "idle_or_stopped")
    ) {
      throw new Error("Invalid Provider configuration option presentation");
    }
    if (optionPresentationIds.has(raw.id)) {
      throw new Error("Duplicate Provider configuration option presentation");
    }
    optionPresentationIds.add(raw.id);
  }
  if (input.ui.assets.length > 64) {
    throw new Error("Too many Provider UI assets");
  }
  const assets = new Set<string>();
  const assetRoles = new Map<string, string>();
  for (const raw of input.ui.assets) {
    validateUiAsset(raw, input.ui.schema_version);
    if (assets.has(raw.id)) throw new Error("Duplicate Provider UI asset");
    assets.add(raw.id);
    assetRoles.set(raw.id, raw.role);
  }
  if (
    assetRoles.get(input.display.logo_asset) !== "logo" ||
    assetRoles.get(input.display.icon_asset) !== "icon" ||
    ![...assetRoles.values()].includes("loading")
  ) {
    throw new Error("Provider display assets have invalid roles");
  }
  if (
    input.logic.state.length > 64 || input.logic.messages.length > 128 ||
    input.logic.reducers.length > 256 || input.logic.effects.length > 64
  ) {
    throw new Error("Provider logic exceeds resource limits");
  }
  const state = new Map<string, ValueType>();
  for (const raw of input.logic.state) {
    if (
      !isRecord(raw) || typeof raw.id !== "string" || !isIdentifier(raw.id) ||
      !isValueType(raw.value_type) ||
      !matchesType(raw.initial, raw.value_type)
    ) throw new Error("Invalid Provider state field");
    if (state.has(raw.id)) throw new Error("Duplicate Provider state field");
    state.set(raw.id, raw.value_type);
  }
  const messages = new Map<string, MessageSchema>();
  for (const raw of input.logic.messages) {
    if (
      !isRecord(raw) || typeof raw.id !== "string" || !isIdentifier(raw.id) ||
      !isRecord(raw.payload) ||
      Object.keys(raw.payload).length > 64 ||
      !Object.keys(raw.payload).every(isIdentifier)
    ) {
      throw new Error("Invalid Provider message schema");
    }
    if (!Object.values(raw.payload).every(isValueType)) {
      throw new Error("Invalid Provider message payload type");
    }
    if (messages.has(raw.id)) {
      throw new Error("Duplicate Provider message schema");
    }
    messages.set(raw.id, raw as unknown as MessageSchema);
  }
  const effects = new Set<string>();
  for (const raw of input.logic.effects) {
    if (
      !isRecord(raw) || typeof raw.id !== "string" || !isIdentifier(raw.id) ||
      effects.has(raw.id) ||
      !EFFECT_CAPABILITIES.has(String(raw.capability)) ||
      !isRecord(raw.request) ||
      Object.keys(raw.request).length > 64 ||
      !Object.keys(raw.request).every(isIdentifier) ||
      !Object.values(raw.request).every(isValueType) ||
      typeof raw.success_message !== "string" ||
      typeof raw.failure_message !== "string"
    ) throw new Error("Invalid Provider effect");
    effects.add(raw.id);
    if (
      !messages.has(raw.success_message) || !messages.has(raw.failure_message)
    ) {
      throw new Error("Provider effect completion message is undeclared");
    }
  }
  const effectMessages = new Set<string>();
  for (const raw of input.logic.reducers) {
    if (
      !isRecord(raw) || typeof raw.message !== "string" ||
      !messages.has(raw.message) ||
      !Array.isArray(raw.assignments) || raw.assignments.length > 64 ||
      (raw.effect !== undefined && typeof raw.effect !== "string")
    ) {
      throw new Error("Invalid Provider reducer");
    }
    if (typeof raw.effect === "string") {
      if (!effects.has(raw.effect)) {
        throw new Error("Provider reducer references unknown effect");
      }
      if (effectMessages.has(raw.message)) {
        throw new Error("Provider message invokes multiple effects");
      }
      effectMessages.add(raw.message);
    }
    for (const assignment of raw.assignments) {
      if (!isRecord(assignment) || typeof assignment.field !== "string") {
        throw new Error("Invalid Provider assignment");
      }
      const targetType = state.get(assignment.field);
      const value = assignment.value;
      if (!targetType || !isRecord(value) || typeof value.source !== "string") {
        throw new Error("Provider assignment type mismatch");
      }
      const sourceType = value.source === "literal"
        ? literalType(value.value)
        : value.source === "state" && typeof value.field === "string"
        ? state.get(value.field)
        : value.source === "message" && typeof value.field === "string"
        ? messages.get(raw.message)?.payload[value.field]
        : undefined;
      if (!sourceType || sourceType !== targetType) {
        throw new Error("Provider assignment type mismatch");
      }
    }
  }
  const surfaces = input.ui.surfaces;
  if (
    Object.keys(surfaces).some((slot) =>
      !SURFACE_SLOTS.includes(slot as SurfaceSlot)
    )
  ) {
    throw new Error("Provider declares an unknown UI surface");
  }
  const nodes = { count: 0 };
  for (const slot of SURFACE_SLOTS) {
    if (!(slot in surfaces)) {
      throw new Error(`Provider is missing ${slot} surface`);
    }
    validateUiNode(
      surfaces[slot],
      input.ui.schema_version,
      state,
      messages,
      assets,
      0,
      nodes,
    );
    validateSurfaceEffectOwnership(
      slot,
      surfaces[slot] as UiNode,
      input as unknown as ProviderUiManifest,
    );
  }
  if (nodes.count > 2_048) throw new Error("Provider UI has too many nodes");
}

const SURFACE_SLOTS: readonly SurfaceSlot[] = [
  "card",
  "setup",
  "settings",
  "information",
  "empty",
  "loading",
  "error",
  "session",
];

const TONES = new Set(["neutral", "primary", "success", "warning", "error"]);
const HOST_TEXT_FIELDS = new Set([
  "provider_version",
  "installation_state",
  "authentication_state",
  "distribution_state",
  "machine_name",
  "error_detail",
]);
const HOST_BOOL_FIELDS = new Set([
  "installed",
  "auth_ready",
  "auth_required",
  "machine_online",
  "upgrade_available",
]);
const EFFECT_CAPABILITIES: ReadonlySet<string> = new Set([
  "begin_service_authentication",
  "logout_service_authentication",
  "install_on_machine",
  "upgrade_on_machine",
  "request_uninstall_plan",
  "open_external_documentation",
]);

function validateSurfaceEffectOwnership(
  slot: SurfaceSlot,
  node: UiNode,
  manifest: ProviderUiManifest,
): void {
  if (node.component === "stack") {
    for (const child of node.children) {
      validateSurfaceEffectOwnership(slot, child, manifest);
    }
    return;
  }
  if (node.component !== "button") return;
  for (
    const reducer of manifest.logic.reducers.filter((rule) =>
      rule.message === node.emit.message && rule.effect !== undefined
    )
  ) {
    const effect = manifest.logic.effects.find((candidate) =>
      candidate.id === reducer.effect
    );
    if (!effect) {
      throw new Error("Provider surface button references an unknown effect");
    }
    const valid = effect.capability === "open_external_documentation" ||
      ((effect.capability === "begin_service_authentication" ||
        effect.capability === "logout_service_authentication") &&
        slot === "setup") ||
      (effect.capability === "install_on_machine" && slot === "empty") ||
      ((effect.capability === "upgrade_on_machine" ||
        effect.capability === "request_uninstall_plan") && slot === "settings");
    if (!valid) {
      throw new Error(
        `Provider effect ${effect.capability} is not allowed on ${slot} surface`,
      );
    }
  }
}

function validateUiAsset(
  input: unknown,
  uiSchemaVersion: number,
): asserts input is UiAsset {
  if (
    !isRecord(input) || typeof input.id !== "string" ||
    !isIdentifier(input.id) ||
    !["logo", "icon", "loading", "illustration"].includes(String(input.role)) ||
    typeof input.media_type !== "string" || typeof input.digest !== "string" ||
    !isDigest(input.digest) ||
    typeof input.accessible_label !== "string" ||
    !input.accessible_label.trim() ||
    input.accessible_label.length > 512 || !isRecord(input.content)
  ) {
    throw new Error("Invalid Provider UI asset");
  }
  if (input.content.kind === "vector_path") {
    if (
      input.media_type !== "image/svg+xml" ||
      typeof input.content.view_box !== "string" ||
      !isViewBox(input.content.view_box) ||
      typeof input.content.path !== "string" ||
      !isVectorPath(input.content.path) ||
      (input.content.fill !== undefined &&
        (typeof input.content.fill !== "string" ||
          !isColor(input.content.fill))) ||
      (input.content.gradient !== undefined &&
        (uiSchemaVersion < 2 ||
          !isVectorGradient(input.content.gradient))) ||
      (input.content.fill !== undefined &&
        input.content.gradient !== undefined)
    ) {
      throw new Error("Invalid Provider vector asset");
    }
    return;
  }
  if (input.content.kind === "inline") {
    if (
      !["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif"]
        .includes(input.media_type) ||
      typeof input.content.base64 !== "string" ||
      !isBoundedBase64(input.content.base64, 1_048_576)
    ) {
      throw new Error("Invalid Provider inline asset");
    }
    return;
  }
  throw new Error("Unknown Provider UI asset content");
}

function validateOptionalExpression(
  input: unknown,
  state: ReadonlyMap<string, ValueType>,
  depth: number,
): void {
  if (input === undefined) return;
  if (depth > 16 || !isRecord(input) || typeof input.op !== "string") {
    throw new Error("Invalid Provider boolean expression");
  }
  if (input.op === "state_equals" && typeof input.field === "string") {
    const valueType = state.get(input.field);
    if (valueType && matchesType(input.value, valueType)) return;
  } else if (
    input.op === "host_equals" && typeof input.field === "string" &&
    HOST_BOOL_FIELDS.has(input.field) && typeof input.value === "boolean"
  ) {
    return;
  } else if (
    (input.op === "all" || input.op === "any") && Array.isArray(input.values) &&
    input.values.length > 0 && input.values.length <= 32
  ) {
    for (const value of input.values) {
      validateOptionalExpression(value, state, depth + 1);
    }
    return;
  } else if (input.op === "not") {
    validateOptionalExpression(input.value, state, depth + 1);
    if (input.value !== undefined) return;
  }
  throw new Error("Invalid Provider boolean expression");
}

function isTone(value: unknown): value is Tone {
  return typeof value === "string" && TONES.has(value);
}

function isOptionalTone(value: unknown): value is Tone | undefined {
  return value === undefined || isTone(value);
}

function isColor(value: string): boolean {
  return /^#[0-9A-Fa-f]{6}$/.test(value);
}

function isDigest(value: string): boolean {
  return /^sha256:[0-9A-Fa-f]{64}$/.test(value);
}

function isViewBox(value: string): boolean {
  const fields = value.trim().split(/\s+/).map(Number);
  return fields.length === 4 && fields.every(Number.isFinite) &&
    fields[2]! > 0 && fields[3]! > 0;
}

function isVectorPath(value: string): boolean {
  return value.trim().length > 0 && value.length <= 65_536 &&
    /^[MmZzLlHhVvCcSsQqTtAa0-9eE+.,\s-]+$/.test(value);
}

function isVectorGradient(input: unknown): input is VectorGradient {
  if (
    !isRecord(input) ||
    !hasOnlyKeys(input, [
      "x1_percent",
      "y1_percent",
      "x2_percent",
      "y2_percent",
      "stops",
    ]) ||
    ![input.x1_percent, input.y1_percent, input.x2_percent, input.y2_percent]
      .every((value) =>
        typeof value === "number" && Number.isSafeInteger(value) &&
        value >= -100 && value <= 200
      ) ||
    !Array.isArray(input.stops) || input.stops.length < 2 ||
    input.stops.length > 8
  ) return false;
  let previous = -1;
  for (const stop of input.stops) {
    if (
      !isRecord(stop) || typeof stop.offset_percent !== "number" ||
      !hasOnlyKeys(stop, ["offset_percent", "color"]) ||
      !Number.isSafeInteger(stop.offset_percent) ||
      stop.offset_percent < 0 || stop.offset_percent > 100 ||
      stop.offset_percent < previous || typeof stop.color !== "string" ||
      !isColor(stop.color)
    ) return false;
    previous = stop.offset_percent;
  }
  return true;
}

function isBoundedBase64(value: string, maximumBytes: number): boolean {
  if (
    value.length === 0 || value.length % 4 !== 0 ||
    !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(
      value,
    )
  ) {
    return false;
  }
  const padding = value.endsWith("==") ? 2 : value.endsWith("=") ? 1 : 0;
  return value.length / 4 * 3 - padding <= maximumBytes;
}

function isCompatibleContract(
  minimum: unknown,
  maximum: unknown,
  current: number,
): boolean {
  return typeof minimum === "number" && Number.isSafeInteger(minimum) &&
    minimum >= 0 &&
    typeof maximum === "number" && Number.isSafeInteger(maximum) &&
    maximum >= minimum &&
    minimum <= current && maximum >= current;
}

function validateUiNode(
  input: unknown,
  uiSchemaVersion: number,
  state: ReadonlyMap<string, ValueType>,
  messages: ReadonlyMap<string, MessageSchema>,
  assets: ReadonlySet<string>,
  depth: number,
  nodes: { count: number },
): asserts input is UiNode {
  if (depth > 32) throw new Error("Provider UI is too deeply nested");
  nodes.count += 1;
  if (!isRecord(input) || typeof input.component !== "string") {
    throw new Error("Invalid Provider UI node");
  }
  switch (input.component) {
    case "stack":
      if (
        !["row", "column", "responsive"].includes(String(input.direction)) ||
        !["xs", "sm", "md", "lg"].includes(String(input.gap)) ||
        (input.wrap !== undefined && typeof input.wrap !== "boolean") ||
        (uiSchemaVersion < 2 && input.wrap === true) ||
        !Array.isArray(input.children) || input.children.length > 64
      ) {
        throw new Error("Invalid Provider stack");
      }
      validateOptionalExpression(input.visible_when, state, 0);
      for (const child of input.children) {
        validateUiNode(
          child,
          uiSchemaVersion,
          state,
          messages,
          assets,
          depth + 1,
          nodes,
        );
      }
      break;
    case "text":
      if (
        !["title", "body", "caption", "code"].includes(String(input.variant)) ||
        !isOptionalTone(input.tone)
      ) throw new Error("Invalid Provider text node");
      validateText(input.value, state);
      break;
    case "asset":
      if (
        typeof input.asset !== "string" || !assets.has(input.asset) ||
        !["sm", "md", "lg", "fill"].includes(String(input.size))
      ) {
        throw new Error("Invalid Provider asset node");
      }
      break;
    case "badge":
      if (!isTone(input.tone)) throw new Error("Invalid Provider badge");
      validateText(input.label, state);
      break;
    case "progress":
      validateText(input.label, state);
      break;
    case "activity":
      if (uiSchemaVersion < 2) {
        throw new Error("Provider activity requires UI schema 2");
      }
      validateActivityNode(input, state, assets);
      break;
    case "alert":
      if (!isTone(input.tone)) throw new Error("Invalid Provider alert");
      validateText(input.title, state);
      validateText(input.body, state);
      break;
    case "divider":
      break;
    case "button":
      if (
        !["primary", "secondary", "destructive"].includes(String(input.style))
      ) {
        throw new Error("Invalid Provider button style");
      }
      validateText(input.label, state);
      validateOptionalExpression(input.enabled_when, state, 0);
      if (
        !isRecord(input.emit) || typeof input.emit.message !== "string" ||
        !isRecord(input.emit.payload) ||
        !Object.values(input.emit.payload).every((value) =>
          literalType(value) !== undefined
        )
      ) {
        throw new Error("Invalid Provider button emission");
      }
      const message = messages.get(input.emit.message);
      if (!message) throw new Error("Provider button emits an unknown message");
      validatePayload(
        message,
        input.emit.payload as Record<string, LiteralValue>,
      );
      break;
    default:
      throw new Error(`Unknown Provider UI component ${input.component}`);
  }
}

function validateActivityNode(
  input: Record<string, unknown>,
  state: ReadonlyMap<string, ValueType>,
  assets: ReadonlySet<string>,
): void {
  if (
    !hasOnlyKeys(input, [
      "component",
      "indicator",
      "label",
      "accessible_label",
    ]) ||
    typeof input.accessible_label !== "string" ||
    !input.accessible_label.trim() || input.accessible_label.length > 512 ||
    !isRecord(input.indicator) || !isRecord(input.label)
  ) throw new Error("Invalid Provider activity");
  const indicator = input.indicator;
  const interval = indicator.interval_ms;
  switch (indicator.kind) {
    case "progress_ring":
      if (!hasOnlyKeys(indicator, ["kind"])) {
        throw new Error("Invalid Provider progress activity");
      }
      break;
    case "glyph_cycle":
      if (
        !hasOnlyKeys(indicator, ["kind", "frames", "interval_ms"]) ||
        !isIntegerBetween(interval, 120, 10_000) ||
        !Array.isArray(indicator.frames) || indicator.frames.length < 2 ||
        indicator.frames.length > 16 ||
        !indicator.frames.every((frame) =>
          typeof frame === "string" && frame.length > 0 &&
          [...frame].length <= 8 && !hasControlCharacter(frame)
        )
      ) throw new Error("Invalid Provider glyph activity");
      break;
    case "terminal_prompt":
      if (
        !hasOnlyKeys(indicator, ["kind", "interval_ms"]) ||
        !isIntegerBetween(interval, 400, 10_000)
      ) {
        throw new Error("Invalid Provider terminal activity");
      }
      break;
    case "asset_signal":
    case "asset_pulse":
      if (
        !hasOnlyKeys(indicator, ["kind", "asset", "interval_ms"]) ||
        typeof indicator.asset !== "string" ||
        !assets.has(indicator.asset) ||
        !isIntegerBetween(interval, 400, 10_000)
      ) throw new Error("Invalid Provider asset activity");
      break;
    default:
      throw new Error("Unknown Provider activity indicator");
  }
  const label = input.label;
  if (
    label.kind === "text" &&
    !hasOnlyKeys(label, ["kind", "value", "effect"])
  ) throw new Error("Invalid Provider activity text label");
  if (!["none", "fade", "shimmer"].includes(String(label.effect))) {
    throw new Error("Invalid Provider activity text effect");
  }
  if (label.kind === "text") {
    validateText(label.value, state);
    return;
  }
  if (
    label.kind !== "phrase_cycle" ||
    !hasOnlyKeys(label, [
      "kind",
      "phrases",
      "interval_ms",
      "suffix",
      "effect",
    ]) ||
    !Array.isArray(label.phrases) || label.phrases.length < 1 ||
    label.phrases.length > 64 ||
    !label.phrases.every((phrase) =>
      typeof phrase === "string" && Boolean(phrase.trim()) &&
      phrase.length <= 128 && !hasControlCharacter(phrase)
    ) ||
    !isIntegerBetween(label.interval_ms, 500, 60_000) ||
    typeof label.suffix !== "string" || [...label.suffix].length > 8
  ) throw new Error("Invalid Provider activity label");
}

function isIntegerBetween(
  value: unknown,
  minimum: number,
  maximum: number,
): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) &&
    value >= minimum && value <= maximum;
}

function hasControlCharacter(value: string): boolean {
  return [...value].some((character) => {
    const code = character.codePointAt(0) ?? 0;
    return code < 0x20 || code === 0x7f;
  });
}

function validateText(
  input: unknown,
  state: ReadonlyMap<string, ValueType>,
): asserts input is TextValue {
  if (!isRecord(input) || typeof input.source !== "string") {
    throw new Error("Invalid Provider text");
  }
  if (
    input.source === "literal" && typeof input.value === "string" &&
    input.value.length <= 4_096
  ) return;
  if (
    input.source === "state" && typeof input.field === "string" &&
    state.has(input.field)
  ) return;
  if (
    input.source === "host" && typeof input.field === "string" &&
    HOST_TEXT_FIELDS.has(input.field)
  ) return;
  throw new Error("Invalid Provider text source");
}

function validatePayload(
  schema: MessageSchema,
  payload: Record<string, LiteralValue>,
): void {
  const expected = Object.keys(schema.payload).sort();
  const actual = Object.keys(payload).sort();
  if (
    expected.length !== actual.length ||
    expected.some((key, index) => key !== actual[index])
  ) {
    throw new Error(`Provider message ${schema.id} payload shape mismatch`);
  }
  for (const [key, type] of Object.entries(schema.payload)) {
    if (!matchesType(payload[key], type)) {
      throw new Error(`Provider message ${schema.id}.${key} type mismatch`);
    }
  }
}

function isValueType(value: unknown): value is ValueType {
  return value === "string" || value === "bool" || value === "integer";
}

function matchesType(value: unknown, type: ValueType): value is LiteralValue {
  if (type === "string") return typeof value === "string";
  if (type === "bool") return typeof value === "boolean";
  return typeof value === "number" && Number.isSafeInteger(value);
}

function literalType(value: unknown): ValueType | undefined {
  if (typeof value === "string") return "string";
  if (typeof value === "boolean") return "bool";
  if (typeof value === "number" && Number.isSafeInteger(value)) {
    return "integer";
  }
  return undefined;
}

function isRuntimeValue(value: unknown): value is RuntimeValue {
  if (typeof value === "string") {
    return value.length <= 32_768 && !value.includes("\0");
  }
  if (!isRecord(value)) return false;
  const decorationsAreSafe = (value.prefix === undefined ||
    (typeof value.prefix === "string" && value.prefix.length <= 4_096 &&
      !value.prefix.includes("\0"))) &&
    (value.suffix === undefined ||
      (typeof value.suffix === "string" && value.suffix.length <= 4_096 &&
        !value.suffix.includes("\0")));
  if (!decorationsAreSafe) return false;
  if (value.source === "sidecar_url") {
    return typeof value.sidecar === "string" && isIdentifier(value.sidecar) &&
      Object.keys(value).every((key) =>
        ["source", "sidecar", "prefix", "suffix"].includes(key)
      );
  }
  return value.source === "component_command" &&
    isRuntimeComponentReference(value.component) &&
    Object.keys(value).every((key) =>
      ["source", "component", "prefix", "suffix"].includes(key)
    );
}

function isRuntimeComponentReference(
  value: unknown,
): value is RuntimeComponentReference {
  return isRecord(value) &&
    ["provider_cli", "provider_adapter", "provider_gateway", "acp_runtime"]
      .includes(String(value.kind)) &&
    typeof value.slot === "string" && isIdentifier(value.slot) &&
    Object.keys(value).every((key) => key === "kind" || key === "slot");
}

function isEnvironmentName(value: string): boolean {
  return /^[A-Z_][A-Z0-9_]*$/.test(value);
}

function isSafeHealthPath(value: string): boolean {
  return value.startsWith("/") && !value.startsWith("//") &&
    value.length <= 256 && !/[?#\0]/.test(value);
}

function isIdentifier(value: string): boolean {
  return value.length > 0 && value.length <= 128 && value !== "." &&
    value !== ".." &&
    /^[a-z0-9._-]+$/.test(value);
}

function isExactVersion(value: string): boolean {
  return value.length > 0 && value.length <= 128 &&
    value.toLowerCase() !== "latest" &&
    !/[\^~*<>=]/.test(value) && /^[A-Za-z0-9.+-]+$/.test(value);
}

interface SemanticVersion {
  major: bigint;
  minor: bigint;
  patch: bigint;
  prerelease: string[];
}

export function compareProviderVersions(left: string, right: string): number {
  const a = parseSemanticVersion(left);
  const b = parseSemanticVersion(right);
  if (!a || !b) throw new Error("Provider version is not valid SemVer");
  for (const field of ["major", "minor", "patch"] as const) {
    if (a[field] !== b[field]) return a[field] < b[field] ? -1 : 1;
  }
  if (a.prerelease.length === 0 || b.prerelease.length === 0) {
    return a.prerelease.length === b.prerelease.length
      ? 0
      : a.prerelease.length === 0
      ? 1
      : -1;
  }
  for (
    let index = 0;
    index < Math.max(a.prerelease.length, b.prerelease.length);
    index += 1
  ) {
    const leftPart = a.prerelease[index];
    const rightPart = b.prerelease[index];
    if (leftPart === undefined || rightPart === undefined) {
      return leftPart === undefined ? -1 : 1;
    }
    if (leftPart === rightPart) continue;
    const leftNumeric = /^\d+$/.test(leftPart);
    const rightNumeric = /^\d+$/.test(rightPart);
    if (leftNumeric && rightNumeric) {
      return BigInt(leftPart) < BigInt(rightPart) ? -1 : 1;
    }
    if (leftNumeric !== rightNumeric) return leftNumeric ? -1 : 1;
    return leftPart < rightPart ? -1 : 1;
  }
  return 0;
}

export function validateProviderContractInventory(
  input: unknown,
): ProviderContractInventory {
  const interval = (minimum: unknown, maximum: unknown): boolean =>
    typeof minimum === "number" && Number.isSafeInteger(minimum) &&
    typeof maximum === "number" && Number.isSafeInteger(maximum) &&
    minimum > 0 && minimum <= maximum;
  if (
    !isRecord(input) ||
    !hasOnlyKeys(input, [
      "provider_sdk_version",
      "min_package_schema",
      "max_package_schema",
      "min_release_schema",
      "max_release_schema",
      "min_ui_schema",
      "max_ui_schema",
      "min_host_schema",
      "max_host_schema",
      "machine_contract",
    ]) ||
    typeof input.provider_sdk_version !== "string" ||
    !parseSemanticVersion(input.provider_sdk_version) ||
    !interval(input.min_package_schema, input.max_package_schema) ||
    !interval(input.min_release_schema, input.max_release_schema) ||
    !interval(input.min_ui_schema, input.max_ui_schema) ||
    !interval(input.min_host_schema, input.max_host_schema) ||
    typeof input.machine_contract !== "number" ||
    !Number.isSafeInteger(input.machine_contract) ||
    input.machine_contract < 1
  ) {
    throw new Error("Invalid Machine Provider capability inventory");
  }
  return input as unknown as ProviderContractInventory;
}

export function providerCompatibilityProblem(
  entry: ProviderCatalogEntry,
  target: ProviderCompatibilityTarget,
): ProviderCompatibilityProblem | undefined {
  if (target.provider_contracts === undefined) {
    return {
      code: "capability_inventory_unavailable",
      detail:
        "This Cowboy Machine predates Provider compatibility negotiation. Update Cowboy Machine before installing or upgrading Providers.",
    };
  }
  let inventory: ProviderContractInventory;
  try {
    inventory = validateProviderContractInventory(target.provider_contracts);
  } catch {
    return {
      code: "capability_inventory_invalid",
      detail:
        "Cowboy Machine reported an invalid Provider capability inventory. Update Cowboy Machine before installing or upgrading Providers.",
    };
  }
  const display = entry.manifest.display.name;
  const version = entry.provider_version;
  const update = (requirement: string): string =>
    `${display} ${version} requires ${requirement}. Update Cowboy Machine before installing or upgrading this Provider.`;
  if (
    PROVIDER_PACKAGE_SCHEMA_VERSION < inventory.min_package_schema ||
    PROVIDER_PACKAGE_SCHEMA_VERSION > inventory.max_package_schema
  ) {
    return {
      code: "package_schema_unsupported",
      detail: update(
        `Provider package schema ${PROVIDER_PACKAGE_SCHEMA_VERSION}`,
      ),
    };
  }
  if (
    PROVIDER_RELEASE_SCHEMA_VERSION < inventory.min_release_schema ||
    PROVIDER_RELEASE_SCHEMA_VERSION > inventory.max_release_schema
  ) {
    return {
      code: "release_schema_unsupported",
      detail: update(
        `Provider release schema ${PROVIDER_RELEASE_SCHEMA_VERSION}`,
      ),
    };
  }
  const providerSdk = parseSemanticVersion(entry.manifest.sdk_version);
  const machineSdk = parseSemanticVersion(inventory.provider_sdk_version);
  if (
    providerSdk === null || machineSdk === null ||
    providerSdk.major !== machineSdk.major ||
    compareProviderVersions(
        entry.manifest.sdk_version,
        inventory.provider_sdk_version,
      ) > 0
  ) {
    return {
      code: "provider_sdk_unsupported",
      detail: update(`Cowboy Provider SDK ${entry.manifest.sdk_version}`),
    };
  }
  if (
    entry.manifest.ui.schema_version < inventory.min_ui_schema ||
    entry.manifest.ui.schema_version > inventory.max_ui_schema
  ) {
    return {
      code: "ui_schema_unsupported",
      detail: update(`Provider UI schema ${entry.manifest.ui.schema_version}`),
    };
  }
  if (
    entry.manifest.host.schema_version < inventory.min_host_schema ||
    entry.manifest.host.schema_version > inventory.max_host_schema
  ) {
    return {
      code: "host_schema_unsupported",
      detail: update(
        `Provider host integration schema ${entry.manifest.host.schema_version}`,
      ),
    };
  }
  if (
    inventory.machine_contract <
      entry.manifest.compatibility.min_machine_contract ||
    inventory.machine_contract >
      entry.manifest.compatibility.max_machine_contract
  ) {
    return {
      code: "machine_contract_unsupported",
      detail: update(
        `Machine Provider contract ${entry.manifest.compatibility.min_machine_contract}`,
      ),
    };
  }
  if (
    !entry.supported_platforms.some((candidate) =>
      candidate.os === target.platform &&
      candidate.architecture === target.architecture
    )
  ) {
    return {
      code: "platform_unsupported",
      detail:
        `${display} ${version} is not published for this Cowboy Machine platform.`,
    };
  }
  return undefined;
}

function parseSemanticVersion(value: string): SemanticVersion | null {
  if (value.length === 0 || value.length > 128) return null;
  const match =
    /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/
      .exec(value);
  if (!match) return null;
  const prerelease = match[4]?.split(".") ?? [];
  if (
    prerelease.some((part) =>
      /^\d+$/.test(part) && part.length > 1 && part.startsWith("0")
    )
  ) {
    return null;
  }
  const core = [
    BigInt(match[1]!),
    BigInt(match[2]!),
    BigInt(match[3]!),
  ] as const;
  if (core.some((part) => part > 18_446_744_073_709_551_615n)) return null;
  return {
    major: core[0],
    minor: core[1],
    patch: core[2],
    prerelease,
  };
}

function isCompatibleProviderSdkVersion(value: string): boolean {
  const provider = parseSemanticVersion(value);
  const supported = parseSemanticVersion(PROVIDER_SDK_VERSION);
  return provider !== null && supported !== null &&
    provider.major === supported.major &&
    compareProviderVersions(value, PROVIDER_SDK_VERSION) <= 0;
}

function isSafeRelativePath(value: string): boolean {
  return value.length > 0 && !value.startsWith("/") &&
    value.split("/").every((segment) =>
      segment.length > 0 && segment !== "." && segment !== ".."
    );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasOnlyKeys(
  value: Record<string, unknown>,
  allowed: readonly string[],
): boolean {
  const allowedKeys = new Set(allowed);
  return Object.keys(value).every((key) => allowedKeys.has(key));
}

export function assertNever(value: never): never {
  throw new Error(
    `Unhandled Provider contract variant: ${JSON.stringify(value)}`,
  );
}
