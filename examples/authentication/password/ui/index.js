/* Password login.method slot. Uses the host React/MUI singleton. */
function host() {
  const value = globalThis.__COWBOY_PLUGIN_HOST;
  if (!value) throw new Error("Cowboy plugin host is not installed");
  return value;
}

function errorMessage(err) {
  return err instanceof Error ? err.message : "Could not reach Cowboy";
}

async function submitPassword(context) {
  if (context.busy || !context.canSubmit) return;
  const auth = host().auth;
  if (!auth) {
    context.onSubmit?.();
    return;
  }
  context.onBusy?.(true);
  context.onError?.(null);
  try {
    if (context.mode === "setup") {
      context.onStatus?.(await auth.setup(String(context.setupToken ?? "").trim()));
    } else if (context.mode === "register") {
      context.onAuthed?.(await auth.register(context.account, context.password));
    } else {
      context.onAuthed?.(await auth.login(context.account, context.password));
    }
  } catch (err) {
    context.onError?.(errorMessage(err));
  } finally {
    context.onBusy?.(false);
  }
}

export default function PasswordLogin(props) {
  const context = props.context;
  if (!context || context.kind !== "password") {
    throw new Error("password login context is missing");
  }
  const { React, ui, icons, components } = host();
  const h = React.createElement;
  const fields = [];
  if (context.mode === "setup") {
    fields.push(
      h(ui.TextField, {
        key: "setup",
        label: context.fieldLabels?.setup ?? "Setup code",
        value: context.setupToken,
        onChange: (event) => context.onSetupToken(event.target.value),
        autoComplete: "one-time-code",
        fullWidth: true,
      }),
    );
  } else {
    fields.push(
      h(ui.TextField, {
        key: "account",
        label: context.fieldLabels?.account ?? "Account",
        name: "username",
        value: context.account,
        onChange: (event) => context.onAccount(event.target.value),
        autoComplete: "username",
        autoCapitalize: "none",
        autoCorrect: "off",
        spellCheck: false,
        fullWidth: true,
      }),
      h(ui.TextField, {
        key: "password",
        label: context.fieldLabels?.secret ?? "Password",
        name: context.mode === "register" ? "new-password" : "password",
        type: context.passwordVisible ? "text" : "password",
        value: context.password,
        onChange: (event) => context.onPassword(event.target.value),
        autoComplete: context.mode === "register" ? "new-password" : "current-password",
        error: context.mode === "register" && context.password.length > 0 &&
          !context.passwordAcceptable,
        fullWidth: true,
        slotProps: context.mode === "register"
          ? {
            input: {
              endAdornment: h(
                ui.InputAdornment,
                { position: "end" },
                h(
                  ui.IconButton,
                  {
                    "aria-label": context.passwordVisible
                      ? "Hide password"
                      : "Show password",
                    edge: "end",
                    onClick: context.onTogglePasswordVisible,
                  },
                  h(
                    context.passwordVisible ? icons.VisibilityOff : icons.Visibility,
                  ),
                ),
              ),
            },
            htmlInput: {
              passwordrules:
                "minlength: 15; maxlength: 128; required: lower, upper, digit; allowed: [-];",
            },
          }
          : undefined,
      }),
    );
    if (context.mode === "register" && components.PasswordStrength) {
      fields.push(
        h(components.PasswordStrength, {
          key: "strength",
          password: context.password,
          account: context.account,
        }),
      );
      fields.push(
        h(ui.TextField, {
          key: "confirm",
          label: context.fieldLabels?.confirm ?? "Confirm password",
          type: context.passwordVisible ? "text" : "password",
          value: context.confirm,
          onChange: (event) => context.onConfirm(event.target.value),
          autoComplete: "new-password",
          error: context.confirmMismatch,
          helperText: context.confirmMismatch ? "Passwords do not match" : undefined,
          fullWidth: true,
        }),
      );
    }
  }
  fields.push(
    h(
      ui.Button,
      {
        key: "submit",
        type: "button",
        variant: "contained",
        size: "large",
        disabled: context.busy || !context.canSubmit,
        onClick: () => {
          void submitPassword(context);
        },
      },
      context.submitLabel,
    ),
  );
  return h(React.Fragment, null, ...fields);
}

export const slots = { "login.method": PasswordLogin };
