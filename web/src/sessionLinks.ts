import type { SessionContext } from "./sessionContext";
import type { SessionMeta } from "./protocol";

export interface SessionLinkRow {
  id: string;
  title: string;
  kind: "forked" | "continued" | "linked";
  state: "open" | "trash" | "deleted";
  before?: number;
}

/** Every session this conversation came from or has read, most formative
 *  first. A source can be trashed or deleted at any time, so the live lists
 *  decide its state and the stored title keeps it named. */
export function sessionLinks(
  session: Pick<SessionMeta, "continued_from" | "forked_from"> | undefined,
  links: SessionContext["links"],
  sessions: readonly Pick<SessionMeta, "id" | "title">[],
  trashed: readonly Pick<SessionMeta, "id" | "title">[],
): SessionLinkRow[] {
  const row = (
    id: string,
    kind: SessionLinkRow["kind"],
    title: string | undefined,
    before: number | null | undefined,
  ): SessionLinkRow => {
    const live = sessions.find((entry) => entry.id === id);
    const gone = trashed.find((entry) => entry.id === id);
    return {
      id,
      kind,
      state: live ? "open" : gone ? "trash" : "deleted",
      title: live?.title || gone?.title || title || "Deleted session",
      ...(typeof before === "number" && { before }),
    };
  };
  const stored = links ?? [];
  const find = (id: string | undefined) => stored.find((link) => link.id === id);
  const rows: SessionLinkRow[] = [];
  if (session?.forked_from) {
    const link = find(session.forked_from);
    rows.push(row(session.forked_from, "forked", link?.title, link?.before));
  }
  if (session?.continued_from) {
    const link = find(session.continued_from);
    rows.push(row(session.continued_from, "continued", link?.title, link?.before));
  }
  for (const link of stored) {
    if (rows.some((entry) => entry.id === link.id)) continue;
    rows.push(row(link.id, "linked", link.title, link.before));
  }
  return rows;
}

export const SESSION_LINK_LABEL: Record<SessionLinkRow["kind"], string> = {
  forked: "Forked from",
  continued: "Continued from",
  linked: "Linked",
};
