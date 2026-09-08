/** Closed telemetry payload decoder shared by SDK consumers and build checks. */
export interface TelemetryBackendContract {
  schema_version: 1 | 2;
  id: string;
  version: string;
  display_name: string;
  supported_platforms: Array<
    { os: "linux" | "macos"; architecture: "x86_64" | "aarch64" }
  >;
  logs?: TelemetryRoute;
  metrics?: TelemetryRoute;
  traces?: TelemetryRoute;
}

interface TelemetryRoute {
  encoding: "json_lines" | "prometheus_text" | "otlp_http_protobuf";
  path: string;
  query?: Record<string, string>;
}

function object(value: unknown, fields: string[]): Record<string, unknown> {
  if (
    !value || typeof value !== "object" || Array.isArray(value) ||
    Object.keys(value).some((key) => !fields.includes(key))
  ) {
    throw new Error("Invalid telemetry contract object");
  }
  return value as Record<string, unknown>;
}

function route(value: unknown, encoding: TelemetryRoute["encoding"]): void {
  const input = object(value, ["encoding", "path", "query"]);
  if (
    input.encoding !== encoding || typeof input.path !== "string" ||
    input.path.length > 256 || !/^(\/[a-zA-Z0-9_-]+)+$/.test(input.path)
  ) {
    throw new Error("Invalid telemetry lane encoding or path");
  }
  if (input.query !== undefined) {
    const query = object(
      input.query,
      encoding === "json_lines"
        ? ["_stream_fields", "_time_field", "_msg_field"]
        : [],
    );
    if (
      Object.values(query).some((item) =>
        typeof item !== "string" || !/^[a-zA-Z0-9_,]{1,128}$/.test(item)
      )
    ) {
      throw new Error("Invalid telemetry query mapping");
    }
  }
}

export function validateTelemetryBackendContract(
  value: unknown,
): TelemetryBackendContract {
  const input = object(value, [
    "schema_version",
    "id",
    "version",
    "display_name",
    "supported_platforms",
    "logs",
    "metrics",
    "traces",
  ]);
  if (
    ![1, 2].includes(Number(input.schema_version)) ||
    typeof input.schema_version !== "number" || typeof input.id !== "string" ||
    !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(input.id) ||
    typeof input.version !== "string" ||
    !/^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.test(
      input.version,
    ) ||
    typeof input.display_name !== "string" || !input.display_name.trim() ||
    new TextEncoder().encode(input.display_name).length > 128 ||
    /[\p{Cc}]/u.test(input.display_name) ||
    !Array.isArray(input.supported_platforms) ||
    input.supported_platforms.length === 0 ||
    input.supported_platforms.length > 4 ||
    (input.logs === undefined && input.metrics === undefined &&
      input.traces === undefined) ||
    (input.schema_version === 1 && input.traces !== undefined)
  ) throw new Error("Invalid telemetry backend contract");
  const platforms = new Set<string>();
  for (const value of input.supported_platforms) {
    const platform = object(value, ["os", "architecture"]);
    if (
      !["linux", "macos"].includes(String(platform.os)) ||
      !["x86_64", "aarch64"].includes(String(platform.architecture))
    ) throw new Error("Invalid telemetry platform");
    platforms.add(`${String(platform.os)}:${String(platform.architecture)}`);
  }
  if (platforms.size !== input.supported_platforms.length) {
    throw new Error("Duplicate telemetry platform");
  }
  if (input.logs !== undefined) {
    route(
      input.logs,
      input.schema_version === 2 ? "otlp_http_protobuf" : "json_lines",
    );
  }
  if (input.metrics !== undefined) {
    route(
      input.metrics,
      input.schema_version === 2 ? "otlp_http_protobuf" : "prometheus_text",
    );
  }
  if (input.traces !== undefined) route(input.traces, "otlp_http_protobuf");
  return input as unknown as TelemetryBackendContract;
}
