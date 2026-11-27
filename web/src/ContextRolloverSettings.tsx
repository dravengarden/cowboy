import { useCallback, useEffect, useState } from "react";
import { Button, Stack, Switch, Typography } from "@mui/material";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import type { DeepseekCacheProtectionStatus } from "./ComposerSheet";
import { formatIdleMinutesLabel } from "./contextRolloverPreferences";
import { expectHttpOk } from "./httpResponse";

/** `GET /api/sessions/{id}/context-rollover` (src/context_rollover.rs). */
export interface ContextRolloverView {
  policy: {
    enabled: boolean;
    session_override: boolean | null;
    idle_minutes: number | null;
    min_context_tokens: number;
  };
  context_used: number;
  idle_ms: number | null;
  due: boolean;
  recommended_idle_minutes?: number | null;
  policy_restricted?: boolean;
  restriction?: string | null;
}

function contextRolloverUrl(sessionId: string): string {
  return `/api/sessions/${encodeURIComponent(sessionId)}/context-rollover`;
}

export function contextRolloverCaption(view: ContextRolloverView): string {
  if (view.policy_restricted) {
    return (
      view.restriction ??
      "Automatic context renewal is disabled for this session."
    );
  }
  if (view.policy.idle_minutes === null) {
    if (typeof view.recommended_idle_minutes === "number" && view.recommended_idle_minutes > 0) {
      return `Off for this Provider (${formatIdleMinutesLabel(view.recommended_idle_minutes)} preset available in Settings → Agent).`;
    }
    return "Off for this Provider (configure in Settings → Agent).";
  }
  if (view.due) {
    return "The prompt cache has expired: your next message starts a fresh context and reads earlier history on demand.";
  }
  const idle = formatIdleMinutesLabel(view.policy.idle_minutes);
  const tokens = `${Math.round(view.policy.min_context_tokens / 1_000)}k`;
  return view.policy.enabled
    ? `After ${idle} idle, when the context holds ${tokens}+ tokens, the next message starts a fresh context instead of re-sending it uncached.`
    : "Off: after the prompt cache expires, the next message re-sends the whole context uncached.";
}

/** Session control for renewing the agent context after its cache expired. */
export function ContextRolloverSettings({
  sessionId,
  onPolicyChange,
}: {
  sessionId: string;
  onPolicyChange?: (() => void) | undefined;
}): React.JSX.Element | null {
  const [view, setView] = useState<ContextRolloverView | null>(null);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    const controller = new AbortController();
    const refresh = (): void => {
      void fetch(contextRolloverUrl(sessionId), { signal: controller.signal })
        .then(async (response) => {
          await expectHttpOk(response, "Could not load context renewal");
          setView((await response.json()) as ContextRolloverView);
        })
        .catch(() => {
          if (!controller.signal.aborted) setView(null);
        });
    };
    refresh();
    const onPrefsChanged = (event: Event): void => {
      const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
      if (scopes && !scopes.context_rollover) return;
      refresh();
    };
    window.addEventListener(PREFERENCES_CHANGED, onPrefsChanged);
    const interval = globalThis.setInterval(refresh, 60_000);
    return (): void => {
      controller.abort();
      window.removeEventListener(PREFERENCES_CHANGED, onPrefsChanged);
      globalThis.clearInterval(interval);
    };
  }, [sessionId]);
  const save = useCallback(
    (enabled: boolean | null): void => {
      setSaving(true);
      void fetch(contextRolloverUrl(sessionId), {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ enabled }),
      })
        .then(async (response) => {
          await expectHttpOk(response, "Could not save context renewal");
          setView((await response.json()) as ContextRolloverView);
          onPolicyChange?.();
        })
        .catch(() => {})
        .finally(() => setSaving(false));
    },
    [sessionId, onPolicyChange],
  );
  if (!view) return null;
  if (view.policy_restricted || view.policy.idle_minutes === null) {
    return (
      <Stack spacing={0.25} sx={{ pt: 0.5 }}>
        <Typography variant="body2" color="text.secondary">
          Fresh context after cache expiry
        </Typography>
        <Typography variant="caption" color="text.secondary">
          {contextRolloverCaption(view)}
        </Typography>
      </Stack>
    );
  }
  return (
    <Stack spacing={0.25} sx={{ pt: 0.5 }}>
      <Stack direction="row" alignItems="center" justifyContent="space-between">
        <Typography variant="body2">Fresh context after cache expiry</Typography>
        <Switch
          size="small"
          checked={view.policy.enabled}
          disabled={saving}
          onChange={(_, checked): void => save(checked)}
          inputProps={{ "aria-label": "Fresh context after cache expiry" }}
        />
      </Stack>
      <Typography variant="caption" color={view.due ? "warning.main" : "text.secondary"}>
        {contextRolloverCaption(view)}
      </Typography>
      {view.policy.session_override !== null && (
        <Button
          size="small"
          variant="text"
          disabled={saving}
          onClick={(): void => save(null)}
          sx={{ alignSelf: "flex-start", px: 0, minWidth: 0 }}
        >
          Use default
        </Button>
      )}
    </Stack>
  );
}

/** Poll the DeepSeek cache keepalive status while it applies to the session. */
export function useCacheProtectionStatus(
  visible: boolean,
  sessionId: string,
): {
  cacheProtection: DeepseekCacheProtectionStatus | null;
  cacheProtectionUnavailable: boolean;
} {
  const [cacheProtection, setCacheProtection] = useState<DeepseekCacheProtectionStatus | null>(
    null,
  );
  const [cacheProtectionUnavailable, setCacheProtectionUnavailable] = useState(false);
  useEffect(() => {
    if (!visible) {
      setCacheProtection(null);
      setCacheProtectionUnavailable(false);
      return;
    }
    const controller = new AbortController();
    const refresh = (): void => {
      void fetch(`/api/sessions/${encodeURIComponent(sessionId)}/cache-protection`, {
        signal: controller.signal,
      })
        .then(async (response) => {
          await expectHttpOk(response, "Could not load cache protection status");
          const value = (await response.json()) as DeepseekCacheProtectionStatus;
          if (!["protected", "inactive", "disabled"].includes(value.state)) {
            throw new Error("invalid cache status");
          }
          setCacheProtection(value);
          setCacheProtectionUnavailable(false);
        })
        .catch(() => {
          if (!controller.signal.aborted) {
            setCacheProtection(null);
            setCacheProtectionUnavailable(true);
          }
        });
    };
    refresh();
    const interval = globalThis.setInterval(refresh, 60_000);
    return (): void => {
      controller.abort();
      globalThis.clearInterval(interval);
    };
  }, [visible, sessionId]);
  return { cacheProtection, cacheProtectionUnavailable };
}
