import { Fragment } from "react";
import { Divider, Stack, Typography } from "@mui/material";
import { sessionAccountIdentity } from "./accountIdentity";
import type { SessionMeta } from "./protocol";
import {
  mergeSessionOverview,
  type SessionInfoPayload,
  sessionOverviewSections,
} from "./sessionOverview";
import { useStoreSelector } from "./store";
import { useSubscriptionAccounts } from "./subscriptionAccounts";

// A label/value row used by both info surfaces.
export function InfoRow({ k, v }: { k: string; v: string }): React.JSX.Element {
  return (
    <Stack
      direction="row"
      spacing={2}
      sx={{ justifyContent: "space-between", alignItems: "baseline" }}
    >
      <Typography variant="caption" sx={{ color: "text.secondary", flexShrink: 0 }}>
        {k}
      </Typography>
      <Typography variant="body2" sx={{ wordBreak: "break-all", textAlign: "right" }}>
        {v}
      </Typography>
    </Stack>
  );
}

export function SessionOverviewRows({
  info,
  live,
}: {
  info: SessionInfoPayload;
  live?: SessionMeta | null;
}): React.JSX.Element {
  const { data: accounts } = useSubscriptionAccounts();
  // Look up the live store entry by ID so counters-only updates omitted by the
  // session-list chrome comparator never overwrite fresh HTTP info counts.
  const liveSession = useStoreSelector((snapshot) => snapshot.sessionsById.get(info.id));
  const merged = mergeSessionOverview(info, liveSession ?? live);
  const sections = sessionOverviewSections(merged);
  const account = sessionAccountIdentity(accounts, merged);
  sections[0]?.rows.splice(3, 0, { label: "Account", value: account.label });
  return (
    <Stack spacing={1.15} sx={{ mt: 1 }}>
      {sections.map((section, index) => (
        <Fragment key={section.key}>
          {index > 0 && <Divider />}
          {section.rows.map((row) => (
            <InfoRow key={row.label} k={row.label} v={row.value} />
          ))}
        </Fragment>
      ))}
    </Stack>
  );
}
