// One view of a widget app (docs/widget-apps.md §3–§5): the runtime is owned
// by the page-wide registry (appRuntimes.ts) so the inline row and the
// inspector share one app. This view draws the tree with Cowboy components,
// contains any drawing failure to this app, and asks before replacing an app
// the person typed into with the agent's newer code (the registry holds the
// choice, so every view of the app agrees).

import { Component, Fragment, useMemo, type ReactNode } from "react";
import { Alert, Box, Button, Typography, useTheme } from "@mui/material";
import { stopAppRuntime, useAppRuntime } from "./appRuntimes";
import { isText, type AppTreeState } from "./appTree";
import { renderAppElement, type Dispatch } from "./AppComponents";

export type AppNodeProps = {
  code: string;
  props: unknown;
  title: string;
  /** Session, widget and node of the app; null outside a published widget. */
  identity?: string | null | undefined;
  /** Asked to send a message from the running `code`: the host must confirm
   *  before anything is sent. */
  onSend?: ((text: string, code: string) => void) | undefined;
};

function renderTree(tree: AppTreeState, id: number, dispatch: Dispatch): ReactNode {
  const node = tree.nodes.get(id);
  if (!node) return null;
  if (isText(node)) return node.text;
  const children = node.children.map((child) => (
    <Fragment key={child}>{renderTree(tree, child, dispatch)}</Fragment>
  ));
  const label = node.children
    .map((child) => {
      const value = tree.nodes.get(child);
      return isText(value) ? value.text : "";
    })
    .join("");
  return renderAppElement(node, children, label, dispatch);
}

/** A drawing failure stops this app, never the page. */
class AppBoundary extends Component<
  { onError: () => void; children: ReactNode },
  { failed: boolean }
> {
  override state = { failed: false };
  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }
  override componentDidCatch(): void {
    this.props.onError();
  }
  override render(): ReactNode {
    return this.state.failed ? null : this.props.children;
  }
}

type AppViewProps = Omit<AppNodeProps, "identity" | "onSend"> & {
  identity: string;
  onSend?: ((text: string, code: string) => void) | undefined;
};

function AppView({ code, props, title, identity, onSend }: AppViewProps): React.JSX.Element {
  const theme = useTheme();
  // A published app keeps one runtime across code versions (the registry
  // decides which version runs); a local preview is one app per exact source.
  const key = identity.startsWith("local:") ? `${identity}:${code}` : identity;
  const scope = identity.split(":")[0] ?? identity;
  const context = useMemo(
    () => ({ theme: theme.palette.mode, locale: navigator.language, platform: "web" }),
    [theme.palette.mode],
  );
  const app = useAppRuntime(key, scope, { code, props, context }, onSend);
  if (app.waiting) {
    return (
      <Typography variant="caption" color="text.secondary">
        Too many apps are running on this page. Close one to start “{title}”.
      </Typography>
    );
  }
  if (!app.primary && !app.stopped) {
    return (
      <Typography variant="caption" color="text.secondary">
        “{title}” is open in the app view.
      </Typography>
    );
  }
  if (app.stopped) {
    return (
      <Alert
        severity="warning"
        variant="outlined"
        action={
          <Button size="small" onClick={app.reload} sx={{ textTransform: "none" }}>
            Reload
          </Button>
        }
      >
        {app.stopped}
      </Alert>
    );
  }
  return (
    <>
      {app.outdated ? (
        <Alert
          severity="info"
          variant="outlined"
          action={
            <>
              <Button size="small" onClick={app.keep} sx={{ textTransform: "none" }}>
                Keep editing
              </Button>
              <Button size="small" onClick={app.reload} sx={{ textTransform: "none" }}>
                Reload
              </Button>
            </>
          }
        >
          The agent updated this app. Reload discards what you typed.
        </Alert>
      ) : null}
      {app.tree.root.length === 0 ? (
        <Typography variant="caption" color="text.secondary">
          Starting the app…
        </Typography>
      ) : (
        <AppBoundary
          key={key}
          onError={(): void => stopAppRuntime(key, "This app stopped: Cowboy could not draw it.")}
        >
          {app.tree.root.map((id) => (
            <Fragment key={id}>{renderTree(app.tree, id, app.dispatch)}</Fragment>
          ))}
        </AppBoundary>
      )}
    </>
  );
}

export default function AppNode(props: AppNodeProps): React.JSX.Element {
  return (
    <Box data-widget-app={props.title} sx={{ display: "grid", gap: 1, minWidth: 0 }}>
      <AppView {...props} identity={props.identity ?? `local:${props.title}`} />
    </Box>
  );
}
