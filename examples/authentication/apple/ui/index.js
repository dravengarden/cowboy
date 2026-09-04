/* Shared OpenID Connect login.method slot for Authentication Provider plugins. */
function host() {
  const value = globalThis.__COWBOY_PLUGIN_HOST;
  if (!value) throw new Error("Cowboy plugin host is not installed");
  return value;
}

export default function OidcLogin(props) {
  const context = props.context;
  if (!context || context.kind !== "oidc") {
    throw new Error("oidc login context is missing");
  }
  const { React, ui } = host();
  const h = React.createElement;
  const nodes = [
    h(
      ui.Button,
      {
        key: "start",
        type: "button",
        href: context.native ? undefined : context.startUrl,
        onClick: context.native
          ? () => {
            const start = host().auth?.startOidc;
            if (typeof start === "function") {
              void start(context.provider);
              return;
            }
            context.onStart?.();
          }
          : undefined,
        variant: "contained",
        size: "large",
        fullWidth: true,
        disabled: context.native && context.busy,
      },
      context.native && context.busy ? "Waiting for approval…" : context.buttonLabel,
    ),
  ];
  if (context.native && context.busy) {
    nodes.push(
      h(
        ui.Button,
        {
          key: "cancel",
          type: "button",
          variant: "text",
          onClick: () => {
            host().auth?.cancelOidc?.();
            context.onCancel?.();
          },
        },
        "Cancel",
      ),
    );
  }
  nodes.push(h(ui.Divider, { key: "divider" }, "secure redirect"));
  return h(React.Fragment, null, ...nodes);
}

export const slots = { "login.method": OidcLogin };
