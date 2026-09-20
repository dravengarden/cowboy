import type { CSSProperties, ReactNode } from "react";

// The React twin of the static boot shell in index.html (same markup, same
// global `.boot-*` CSS). Every state that used to paint a centred spinner
// before the app exists renders this instead, so the screen never changes
// shape between the document's first frame and the app's first real paint
// (docs/offline-first-sync.md §Boot presentation). Keep the two in step.

const vars = (values: Record<string, string>): CSSProperties => values as CSSProperties;

export function BootSkeleton({ children }: { children?: ReactNode }): React.JSX.Element {
  return (
    <div id="app-splash" className="boot-shell" aria-label="Loading" role="status">
      <div className="boot-rail">
        {[0, 1, 2, 3, 4, 5, 6].map((row) => <i key={row} className="boot-bar" />)}
      </div>
      <div className="boot-main">
        <div className="boot-feed">
          <i className="boot-card" style={vars({ "--h": "88px" })} />
          <i className="boot-card boot-own" style={vars({ "--h": "48px", "--w": "62%" })} />
          <i className="boot-card" style={vars({ "--h": "132px" })} />
          <i className="boot-card" style={vars({ "--h": "56px" })} />
        </div>
        <div className="boot-composer">
          <i className="boot-bar" style={vars({ "--w": "38%" })} />
          <div className="boot-tools">
            {[0, 1, 2, 3, 4, 5, 6].map((tool) => <i key={tool} className="boot-dot" />)}
          </div>
        </div>
        <div className="boot-nav">
          <i className="boot-dot" />
          <i className="boot-dot" style={vars({ "--d": "12px" })} />
          <i className="boot-bar" style={vars({ "--w": "120px" })} />
          <span className="boot-gap" />
          <i className="boot-dot" />
          <i className="boot-dot" />
          <i className="boot-dot" />
        </div>
      </div>
      {children ? <div className="boot-note">{children}</div> : null}
    </div>
  );
}
