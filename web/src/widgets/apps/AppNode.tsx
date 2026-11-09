// One widget app (docs/widget-apps.md §3–§5): starts its sandboxed runtime,
// applies validated render batches to the host tree, grants one render credit
// per animation frame and draws the tree with Cowboy components. A stopped
// app shows why, with Reload.

import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Alert, Box, Button, Typography, useTheme } from "@mui/material";
import { startAppRuntime, type AppRuntime } from "./appSandbox";
import { AppBudgetError, applyAppOps, emptyAppTree, isText, type AppTreeState } from "./appTree";
import { renderAppElement, type Dispatch } from "./AppComponents";

export type AppNodeProps = {
  code: string;
  props: unknown;
  title: string;
  /** Asked to send a message: the host must confirm before anything is sent. */
  onSend?: (text: string) => void;
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

export default function AppNode({ code, props, title, onSend }: AppNodeProps): React.JSX.Element {
  const theme = useTheme();
  const [tree, setTree] = useState<AppTreeState>(emptyAppTree);
  const [stopped, setStopped] = useState<string | null>(null);
  const [run, setRun] = useState(0);
  const runtime = useRef<AppRuntime | null>(null);
  const propsJson = JSON.stringify(props ?? {});
  const latestProps = useRef(propsJson);
  latestProps.current = propsJson;
  const sendRef = useRef(onSend);
  sendRef.current = onSend;
  const context = useMemo(
    () => ({ theme: theme.palette.mode, locale: navigator.language, platform: "web" }),
    [theme.palette.mode],
  );

  useEffect(() => {
    let current = emptyAppTree();
    let frame = 0;
    setTree(current);
    setStopped(null);
    const instance = startAppRuntime(
      { code, props: JSON.parse(latestProps.current) as unknown, context },
      {
        onOps: (json) => {
          try {
            current = applyAppOps(current, json);
            setTree(current);
            frame = requestAnimationFrame(() => instance.credit());
          } catch (error) {
            instance.terminate();
            setStopped(
              `This app stopped: ${error instanceof AppBudgetError ? error.message : "it sent a bad update"}.`,
            );
          }
        },
        onSend: (text) => sendRef.current?.(text),
        onStopped: (reason) => {
          if (reason !== "closed") setStopped(reason);
        },
      },
    );
    runtime.current = instance;
    return () => {
      cancelAnimationFrame(frame);
      instance.terminate();
      runtime.current = null;
    };
    // `context` and props reach a running app through updates below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [code, run]);

  useEffect(() => {
    runtime.current?.props(JSON.parse(propsJson) as unknown);
  }, [propsJson]);
  useEffect(() => {
    runtime.current?.context(context);
  }, [context]);

  const dispatch: Dispatch = (id, name, payload) => runtime.current?.event(id, name, payload);
  return (
    <Box data-widget-app={title} sx={{ display: "grid", gap: 1, minWidth: 0 }}>
      {stopped ? (
        <Alert
          severity="warning"
          variant="outlined"
          action={
            <Button
              size="small"
              onClick={(): void => setRun((n) => n + 1)}
              sx={{ textTransform: "none" }}
            >
              Reload
            </Button>
          }
        >
          {stopped}
        </Alert>
      ) : tree.root.length === 0 ? (
        <Typography variant="caption" color="text.secondary">
          Starting the app…
        </Typography>
      ) : (
        tree.root.map((id) => <Fragment key={id}>{renderTree(tree, id, dispatch)}</Fragment>)
      )}
    </Box>
  );
}
