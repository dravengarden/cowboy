import { Stack, Typography } from "@mui/material";
import type { SessionMeta } from "./protocol";
import { StatusDot } from "./SessionStatusDot";
import { ProviderIcon } from "./ProviderIcon";
import { SessionCallsBadge } from "./SessionCallsBadge";
import { SessionCacheGlyph } from "./SessionOfflineBadges";

export function SessionRowTitle({ session: s, active }: { session: SessionMeta; active: boolean }): React.JSX.Element {
  return <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0 }}>
    <StatusDot status={s.status} backgroundTasks={s.background_tasks} providerUpdate={s.provider_update} resourceWait={s.resource_wait} />
    <ProviderIcon provider={s.provider} providerVersion={s.provider_version} providerDigest={s.provider_generation_digest}
      sx={{ fontSize: "1.125rem", flexShrink: 0 }} />
    <Typography variant="body2" noWrap sx={{ minWidth: 0 }}>{s.title}</Typography>
    <SessionCallsBadge sessionId={s.id} />
    <SessionCacheGlyph sessionId={s.id} active={active} />
  </Stack>;
}
