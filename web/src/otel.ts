import {
  type Attributes,
  type Context,
  ROOT_CONTEXT,
  SpanKind,
  SpanStatusCode,
  trace,
} from "@opentelemetry/api";
import { SeverityNumber } from "@opentelemetry/api-logs";
import {
  ExportResultCode,
  W3CTraceContextPropagator,
} from "@opentelemetry/core";
import {
  ProtobufLogsSerializer,
  ProtobufMetricsSerializer,
  ProtobufTraceSerializer,
} from "@opentelemetry/otlp-transformer";
import { resourceFromAttributes } from "@opentelemetry/resources";
import {
  BatchLogRecordProcessor,
  LoggerProvider,
} from "@opentelemetry/sdk-logs";
import {
  AggregationTemporality,
  AggregationType,
  MeterProvider,
  type MetricData,
  MetricReader,
} from "@opentelemetry/sdk-metrics";
import {
  BatchSpanProcessor,
  ParentBasedSampler,
  TraceIdRatioBasedSampler,
  TracerProvider,
} from "@opentelemetry/sdk-trace";
import {
  OTLP_MAX_BODY,
  type OtlpSignal,
  OtlpTransport,
} from "./otelTransport.ts";
import {
  cleanTelemetryAttributes,
  cleanTelemetryMessage,
} from "./telemetryQueue.ts";

export const DURATION_BUCKETS = [
  0.005,
  0.01,
  0.025,
  0.05,
  0.1,
  0.25,
  0.5,
  1,
  2.5,
  5,
  10,
  30,
  60,
  120,
];
const DURATIONS = [
  "websocket.connect",
  "websocket.reconnect",
  "long_task",
  "navigation",
  "command",
  "first_output",
] as const;
type Duration = typeof DURATIONS[number];
type Level = "debug" | "info" | "warn" | "error";
type Scalar = string | number | boolean | null;
export interface ClientSpan {
  readonly context: Context;
  readonly traceparent: string;
  end(outcome?: "ok" | "error" | "timeout" | "cancelled"): void;
}

function labels(input: Record<string, string>): Attributes {
  const allowed: Record<string, readonly string[]> = {
    connection: ["initial", "reconnect"],
    transport: ["websocket", "http"],
    reason: [
      "initial",
      "close",
      "error",
      "online",
      "visibility",
      "timeout",
      "heartbeat",
      "other",
    ],
    operation: [
      "submit",
      "prompt",
      "open_session",
      "cancel",
      "set_config_option",
      "sync",
    ],
  };
  return Object.fromEntries(
    Object.entries(input).filter(([key]) => key in allowed).map((
      [key, value],
    ) => [key, allowed[key]?.includes(value) ? value : "other"]),
  );
}

/** Explicit, instance-owned providers; no global context/DOM/console patches,
 * resource detection, ambient exporter endpoints, or provider credentials. */
export function createClientOtel(
  transport: OtlpTransport,
  identity: { platform: string; surface: string },
  sampleRate = 0.1,
) {
  let stopped = false;
  const resource = resourceFromAttributes({
    "service.name": "cowboy-web",
    "cowboy.platform": identity.platform,
    "cowboy.surface": identity.surface,
  });
  const emit = <T>(
    signal: OtlpSignal,
    values: T[],
    serialize: (v: T[]) => Uint8Array | undefined,
  ): void => {
    if (stopped || values.length === 0) return;
    const bytes = serialize(values);
    if (bytes && bytes.byteLength > OTLP_MAX_BODY && values.length > 1) {
      const mid = Math.floor(values.length / 2);
      emit(signal, values.slice(0, mid), serialize);
      emit(signal, values.slice(mid), serialize);
    } else transport.enqueue(signal, bytes, values.length);
  };
  const loggerProvider = new LoggerProvider({
    resource,
    logRecordLimits: {
      attributeCountLimit: 16,
      attributeValueLengthLimit: 256,
    },
    processors: [
      new BatchLogRecordProcessor({
        exporter: {
          export(records, done) {
            emit("logs", records, ProtobufLogsSerializer.serializeRequest);
            done({ code: ExportResultCode.SUCCESS });
          },
          forceFlush() {
            return Promise.resolve();
          },
          shutdown() {
            return Promise.resolve();
          },
        },
        maxQueueSize: 64,
        maxExportBatchSize: 16,
        scheduledDelayMillis: 30_000,
        exportTimeoutMillis: 1000,
        ...{ disableAutoFlushOnDocumentHide: true },
      }),
    ],
  });
  const tracerProvider = new TracerProvider({
    resource,
    sampler: new ParentBasedSampler({
      root: new TraceIdRatioBasedSampler(sampleRate),
    }),
    spanLimits: {
      attributeCountLimit: 16,
      attributeValueLengthLimit: 256,
      eventCountLimit: 4,
      linkCountLimit: 0,
      attributePerEventCountLimit: 4,
    },
    spanProcessors: [
      new BatchSpanProcessor({
        exporter: {
          export(spans, done) {
            emit("traces", spans, ProtobufTraceSerializer.serializeRequest);
            done({ code: ExportResultCode.SUCCESS });
          },
          shutdown() {
            return Promise.resolve();
          },
        },
        maxQueueSize: 64,
        maxExportBatchSize: 16,
        scheduledDelayMillis: 30_000,
        exportTimeoutMillis: 1000,
        ...{ disableAutoFlushOnDocumentHide: true },
      }),
    ],
  });
  class DeltaReader extends MetricReader {
    private collecting: Promise<void> | undefined;
    constructor() {
      super({
        aggregationTemporalitySelector: () => AggregationTemporality.DELTA,
        cardinalitySelector: () => 32,
      });
    }
    protected onShutdown(): Promise<void> {
      return Promise.resolve();
    }
    protected onForceFlush(): Promise<void> {
      return this.collecting ??= this.exportDelta().finally(() => {
        this.collecting = undefined;
      });
    }
    private async exportDelta(): Promise<void> {
      if (stopped) return;
      const { resourceMetrics } = await this.collect({ timeoutMillis: 1000 });
      for (const scope of resourceMetrics.scopeMetrics) {
        for (const metric of scope.metrics) {
          // Serialize each instrument independently. SDK cardinality and the
          // transport byte ceiling bound both collection and retry memory.
          const points = metric.dataPoints;
          emit<MetricData["dataPoints"][number]>(
            "metrics",
            points,
            (dataPoints) =>
              ProtobufMetricsSerializer.serializeRequest({
                resource,
                scopeMetrics: [{
                  scope: scope.scope,
                  metrics: [{ ...metric, dataPoints } as typeof metric],
                }],
              }),
          );
        }
      }
    }
  }
  const reader = new DeltaReader();
  const meterProvider = new MeterProvider({
    resource,
    readers: [reader],
    views: [{
      instrumentName: "cowboy.client.*.duration",
      aggregation: {
        type: AggregationType.EXPLICIT_BUCKET_HISTOGRAM,
        options: { boundaries: DURATION_BUCKETS },
      },
      aggregationCardinalityLimit: 32,
    }],
  });
  const meter = meterProvider.getMeter("cowboy.web", "1");
  const counters = {
    reconnects: meter.createCounter("cowboy.client.websocket.reconnects", {
      unit: "{event}",
    }),
    longTasks: meter.createCounter("cowboy.client.long_tasks", {
      unit: "{event}",
    }),
  };
  const durations = Object.fromEntries(
    DURATIONS.map((
      name,
    ) => [
      name,
      meter.createHistogram(`cowboy.client.${name}.duration`, { unit: "s" }),
    ]),
  ) as Record<Duration, ReturnType<typeof meter.createHistogram>>;
  const logger = loggerProvider.getLogger("cowboy.web", "1");
  const tracer = tracerProvider.getTracer("cowboy.web", "1");
  const propagator = new W3CTraceContextPropagator();
  const spans = new Map<ClientSpan, number>();
  const duration = (
    name: Duration,
    milliseconds: number,
    dimensions: Record<string, string> = {},
  ) => {
    if (
      !stopped && Number.isFinite(milliseconds) && milliseconds >= 0 &&
      milliseconds <= 1_800_000
    ) durations[name].record(milliseconds / 1000, labels(dimensions));
  };
  return {
    transport,
    duration,
    metric(
      name: string,
      value: number,
      dimensions: Record<string, string> = {},
    ) {
      if (stopped || !Number.isFinite(value) || value < 0) return;
      if (name === "websocket_reconnect_success") {
        counters.reconnects.add(value, labels(dimensions));
      } else if (name === "long_task_count") counters.longTasks.add(value);
      else {
        const mapping: Record<string, Duration> = {
          websocket_connect_duration_ms: "websocket.connect",
          websocket_reconnect_duration_ms: "websocket.reconnect",
          long_task_duration_ms: "long_task",
          navigation_duration_ms: "navigation",
        };
        const instrument = mapping[name];
        if (instrument) duration(instrument, value, dimensions);
      }
    },
    log(
      level: Level,
      eventName: string,
      message: unknown,
      attributes: Record<string, Scalar> = {},
      operation?: ClientSpan,
    ) {
      if (stopped) return;
      const severity = {
        debug: SeverityNumber.DEBUG,
        info: SeverityNumber.INFO,
        warn: SeverityNumber.WARN,
        error: SeverityNumber.ERROR,
      };
      logger.emit({
        eventName,
        body: cleanTelemetryMessage(message),
        severityNumber: severity[level],
        severityText: level.toUpperCase(),
        attributes: cleanTelemetryAttributes(attributes),
        context: operation?.context ?? ROOT_CONTEXT,
      });
    },
    start(
      name: "connect" | "command" | "first_output",
      attributes: Record<string, string> = {},
      parent?: ClientSpan,
    ): ClientSpan | undefined {
      if (stopped || spans.size >= 32) return;
      const span = tracer.startSpan(`cowboy.client.${name}`, {
        kind: SpanKind.CLIENT,
        attributes: labels(attributes),
      }, parent?.context ?? ROOT_CONTEXT);
      const context = trace.setSpan(parent?.context ?? ROOT_CONTEXT, span);
      const carrier: Record<string, string> = {};
      propagator.inject(context, carrier, {
        set(c, k, v) {
          c[k] = v;
        },
      });
      const handle: ClientSpan = {
        context,
        traceparent: carrier.traceparent ?? "",
        end(outcome = "ok") {
          if (!spans.delete(handle)) return;
          span.setAttribute("outcome", outcome);
          span.setStatus({
            code: outcome === "ok" ? SpanStatusCode.OK : SpanStatusCode.ERROR,
          });
          span.end();
        },
      };
      spans.set(handle, Date.now());
      return handle;
    },
    async collect() {
      if (stopped) return;
      for (const [span, start] of spans) {
        if (Date.now() - start > 5 * 60_000) span.end("timeout");
      }
      await Promise.allSettled([
        loggerProvider.forceFlush(),
        tracerProvider.forceFlush(),
        reader.forceFlush(),
      ]);
    },
    async stop() {
      stopped = true;
      transport.stop();
      for (const span of spans.keys()) span.end("cancelled");
      spans.clear();
      await Promise.allSettled([
        loggerProvider.shutdown(),
        tracerProvider.shutdown(),
        meterProvider.shutdown(),
      ]);
    },
  };
}
