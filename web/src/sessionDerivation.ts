import { type SessionMeta } from "./protocol";

export type TitleMap = Readonly<Record<string, string>>;

export type OrderList = readonly string[];

/** Apply the title + order overlays to the raw session list (title override
 *  wins; then a stable sort by the synced order — ids not in `order` keep their
 *  relative position at the end, mirroring the daemon's `sort_by_id_order`). */
export function deriveSessions(raw: SessionMeta[], titles: TitleMap, order: OrderList): SessionMeta[] {
  const titled = raw.map((s) => {
    const t = titles[s.id];
    return t !== undefined && t !== s.title ? { ...s, title: t } : s;
  });
  if (order.length === 0) return titled;
  const pos = new Map(order.map((id, i): [string, number] => [id, i]));
  return titled
    .map((s, i): { s: SessionMeta; i: number } => ({ s, i }))
    .sort((a, b) => {
      const pa = pos.get(a.s.id) ?? Number.MAX_SAFE_INTEGER;
      const pb = pos.get(b.s.id) ?? Number.MAX_SAFE_INTEGER;
      return pa !== pb ? pa - pb : a.i - b.i;
    })
    .map((x) => x.s);
}
