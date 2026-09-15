/* Native Claude usage protocol and projection. No HTTP or credential reads. */

const MAX_OUTPUT_BYTES = 256 * 1024;
export const REQUEST_ID = "cowboy-account-usage";
export const USAGE_ARGS = [
  "-p",
  "--input-format",
  "stream-json",
  "--output-format",
  "stream-json",
  "--verbose",
  "--no-session-persistence",
  "--strict-mcp-config",
  "--mcp-config",
  '{"mcpServers":{}}',
  "--setting-sources",
  "",
  "--settings",
  '{"disableAllHooks":true}',
  "--tools",
  "",
];

function record(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value
    : undefined;
}

/** Unknown native diagnostics may contain credentials; expose closed messages. */
function nativeFailure(detail) {
  const message = typeof detail === "string" ? detail.toLowerCase() : "";
  if (
    /unauthorized|forbidden|not logged in|not signed in|\b40[13]\b/.test(
      message,
    )
  ) {
    return new Error("Claude Code usage authentication required.");
  }
  if (/unsupported|unknown.*request|method not found/.test(message)) {
    return new Error("Claude Code usage query is unsupported by this version.");
  }
  return new Error("Claude Code usage is temporarily unavailable.");
}

function usageResponse(message) {
  if (message?.type !== "control_response") return undefined;
  const response = record(message.response);
  if (response?.request_id !== REQUEST_ID) return undefined;
  if (response.subtype !== "success") throw nativeFailure(response.error);
  if (!record(response.response)) {
    throw nativeFailure();
  }
  return response.response;
}

/** Bound the stream and child, including a CLI that stays alive after replying.
 * Children inherit the host-owned process group for descendant cleanup.
 */
export async function nativeJson(args, request, { command, env, deadline }) {
  const timeoutMs = deadline - Date.now();
  if (timeoutMs <= 0) throw new Error("Claude Code usage timed out.");
  let child;
  try {
    child = new Deno.Command(command, {
      args,
      env,
      clearEnv: true,
      stdin: request ? "piped" : "null",
      stdout: "piped",
      stderr: "null",
    }).spawn();
  } catch {
    throw new Error("Claude Code usage command could not start.");
  }
  const stop = () => {
    try {
      child.kill("SIGKILL");
    } catch {
      // The child may already have exited.
    }
  };
  let timedOut = false;
  const reader = child.stdout.getReader();
  const timer = setTimeout(() => {
    timedOut = true;
    stop();
    // A descendant may still hold stdout open after the direct child exits.
    void reader.cancel().catch(() => {});
  }, timeoutMs);
  try {
    if (request) {
      const writer = child.stdin.getWriter();
      try {
        await writer.write(
          new TextEncoder().encode(`${JSON.stringify(request)}\n`),
        );
        await writer.close();
      } finally {
        writer.releaseLock();
      }
    }
    const decoder = new TextDecoder();
    let pending = "";
    let bytes = 0;
    for (;;) {
      const { value, done } = await reader.read();
      if (timedOut) throw new Error("Claude Code usage timed out.");
      bytes += value?.byteLength ?? 0;
      if (bytes > MAX_OUTPUT_BYTES) {
        throw nativeFailure();
      }
      pending += decoder.decode(value, { stream: !done });
      if (request) {
        let newline;
        while ((newline = pending.indexOf("\n")) !== -1 || (done && pending)) {
          const line = newline === -1 ? pending : pending.slice(0, newline);
          pending = newline === -1 ? "" : pending.slice(newline + 1);
          if (!line.trim()) continue;
          let message;
          try {
            message = JSON.parse(line);
          } catch {
            throw nativeFailure();
          }
          const response = usageResponse(message);
          if (response !== undefined) return response;
        }
      } else if (done) {
        let status;
        try {
          status = JSON.parse(pending);
        } catch {
          throw nativeFailure();
        }
        if (!record(status) || typeof status.loggedIn !== "boolean") {
          throw nativeFailure();
        }
        const exit = await child.status;
        if (timedOut) throw new Error("Claude Code usage timed out.");
        if (!exit.success && status.loggedIn) throw nativeFailure();
        return status;
      }
      if (done) throw nativeFailure();
    }
  } catch (error) {
    if (timedOut) throw new Error("Claude Code usage timed out.");
    if (error instanceof Error && error.message.startsWith("Claude Code ")) {
      throw error;
    }
    throw nativeFailure();
  } finally {
    clearTimeout(timer);
    stop();
    await Promise.allSettled([reader.cancel(), child.status]);
    reader.releaseLock();
  }
}

const WINDOWS = [
  ["five_hour", undefined, 300],
  ["seven_day", undefined, 10080],
  ["seven_day_opus", "Opus", 10080],
  ["seven_day_sonnet", "Sonnet", 10080],
  ["seven_day_oauth_apps", "OAuth apps", 10080],
  ["seven_day_overage_included", "Extra usage", 10080],
];

function quotaBucket(window, label, minutes) {
  if (window == null) return undefined;
  if (!record(window)) throw nativeFailure();
  if (window.utilization == null) return undefined;
  if (
    typeof window.utilization !== "number" ||
    !Number.isFinite(window.utilization) || window.utilization < 0 ||
    window.utilization > 100
  ) throw nativeFailure();
  const reset = typeof window.resets_at === "string" &&
      /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/
        .test(window.resets_at)
    ? Date.parse(window.resets_at)
    : NaN;
  return {
    ...(label ? { limitName: label } : {}),
    primary: {
      usedPercent: window.utilization,
      ...(minutes ? { windowDurationMins: minutes } : {}),
      ...(Number.isFinite(reset) ? { resetsAt: reset / 1000 } : {}),
    },
  };
}

/** get_usage reports 0–100 percentages and ISO timestamps. */
export function quotaView(response) {
  if (
    !record(response) || typeof response.rate_limits_available !== "boolean"
  ) {
    throw nativeFailure();
  }
  if (!response.rate_limits_available) return {};
  const limits = record(response.rate_limits);
  if (!limits) throw nativeFailure();
  if (
    !WINDOWS.some(([key]) => Object.hasOwn(limits, key)) &&
    !Object.hasOwn(limits, "model_scoped") &&
    !Object.hasOwn(limits, "extra_usage")
  ) {
    throw nativeFailure();
  }
  if (
    limits.model_scoped !== undefined && !Array.isArray(limits.model_scoped)
  ) {
    throw nativeFailure();
  }
  const models = [];
  for (const window of (limits.model_scoped ?? []).slice(0, 32)) {
    const label = typeof window?.display_name === "string"
      ? window.display_name.trim()
      : "";
    if (!label || label.length > 100 || /[\u0000-\u001f\u007f]/.test(label)) {
      continue;
    }
    const bucket = quotaBucket(window, label, 10080);
    if (bucket) models.push([label, bucket]);
  }
  const buckets = {};
  for (const [key, label, minutes] of WINDOWS) {
    if (
      label &&
      models.some(([name]) => name.toLowerCase() === label.toLowerCase())
    ) continue;
    const bucket = quotaBucket(limits[key], label, minutes);
    if (bucket) buckets[`claude-${key}`] = bucket;
  }
  for (const [label, bucket] of models) {
    buckets[`claude-model-${label}`] = bucket;
  }
  if (limits.extra_usage?.is_enabled === true) {
    const bucket = quotaBucket(limits.extra_usage, "Extra usage");
    if (bucket) buckets["claude-extra-usage"] = bucket;
  }
  return { rate_limits: { rateLimitsByLimitId: buckets } };
}
