// One full app per page (docs/widget-apps-dom.md §5, docs/widget-apps-mcp.md
// §8): a full-DOM app and a signed MCP view share it. Ownership is taken when
// Run is tapped, so an earlier start still loading cannot displace a later
// one; taking it stops the previous owner.

type Owner = { stop: () => void };

let current: Owner | null = null;

export type FullAppClaim = {
  /** Whether this run still owns the page's full app. */
  owns: () => boolean;
  release: () => void;
};

export function claimFullApp(stop: () => void): FullAppClaim {
  const mine: Owner = { stop };
  const previous = current;
  current = mine;
  previous?.stop();
  return {
    owns: () => current === mine,
    release: () => {
      if (current === mine) current = null;
    },
  };
}
