import { Alert, Button, Stack, TextField, Typography } from "@mui/material";
import { useCallback, useEffect, useState } from "react";
import {
  AuthApiError,
  authApi,
  type CreatedProductApiToken,
  type ProductApiToken,
} from "./authApi";
import { useProductAuth } from "./ProductAuthGate";

/** Own-row token CRUD. Lives in auth/ so Desktop/Mobile can mount it without
 *  importing store.ts. */
export function ProductTokensPanel(): React.JSX.Element {
  const { me } = useProductAuth();
  const canCreate = me.role !== "viewer";
  const [tokens, setTokens] = useState<ProductApiToken[]>([]);
  const [name, setName] = useState("zed");
  const [created, setCreated] = useState<CreatedProductApiToken | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async (): Promise<void> => {
    const body = await authApi.listTokens();
    setTokens(body.tokens);
  }, []);

  useEffect(() => {
    void load().catch((err: unknown) => {
      setError(err instanceof AuthApiError ? err.message : "Could not load tokens");
    });
  }, [load]);

  const create = (): void => {
    if (busy || !canCreate || name.trim() === "") return;
    setBusy(true);
    setError(null);
    void authApi
      .createToken(name.trim())
      .then(async (token) => {
        setCreated(token);
        setName("");
        await load();
      })
      .catch((err: unknown) => {
        setError(err instanceof AuthApiError ? err.message : "Could not create token");
      })
      .finally(() => setBusy(false));
  };

  const revoke = (id: string): void => {
    if (busy) return;
    setBusy(true);
    setError(null);
    void authApi
      .deleteToken(id)
      .then(async () => {
        if (created?.id === id) setCreated(null);
        await load();
      })
      .catch((err: unknown) => {
        setError(err instanceof AuthApiError ? err.message : "Could not revoke token");
      })
      .finally(() => setBusy(false));
  };

  return (
    <Stack spacing={1.5}>
      <Typography variant="body2" color="text.secondary">
        Personal access tokens for ACP, curl, and <code>serve-acp</code>.
        Set <code>COWBOY_USER_TOKEN</code> or pass <code>--token</code>. The
        secret is shown once.
      </Typography>
      {error && <Alert severity="error">{error}</Alert>}
      {created && (
        <Alert severity="warning">
          Copy this secret now: <code>{created.token}</code>
        </Alert>
      )}
      {canCreate && (
        <Stack direction="row" spacing={1} alignItems="center">
          <TextField
            size="small"
            label="Token name"
            value={name}
            onChange={(event) => setName(event.target.value)}
            fullWidth
          />
          <Button
            variant="contained"
            disabled={busy || name.trim() === ""}
            onClick={create}
          >
            Create
          </Button>
        </Stack>
      )}
      {tokens.length === 0 ? (
        <Typography variant="body2" color="text.secondary">
          No tokens yet.
        </Typography>
      ) : (
        tokens.map((token) => (
          <Stack
            key={token.id}
            direction="row"
            spacing={1}
            alignItems="center"
            justifyContent="space-between"
          >
            <Typography variant="body2" sx={{ fontFamily: "ui-monospace, monospace" }}>
              {token.token_prefix} · {token.name}
            </Typography>
            <Button
              color="inherit"
              size="small"
              disabled={busy}
              onClick={() => revoke(token.id)}
            >
              Revoke
            </Button>
          </Stack>
        ))
      )}
    </Stack>
  );
}
