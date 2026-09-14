import { Alert, Button, Stack, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { productSyncDatabase } from "./productSyncDatabase.ts";
import { PRODUCT_SESSION_END_EVENT } from "./productSessionEnd.ts";

/** An explicit local export, not a migration grant. Never sends a saved draft. */
export function ProductSyncDataNotice(): React.JSX.Element | null {
  const [keys, setKeys] = useState<string[]>([]);
  const [error, setError] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [busy, setBusy] = useState(false);
  const [page, setPage] = useState(0);
  const lifetime = useRef<
    {
      active: boolean;
      busy: boolean;
      urls: Map<string, ReturnType<typeof setTimeout>>;
    } | null
  >(null);
  useEffect(() => {
    const owned = {
      active: true,
      busy: false,
      urls: new Map<string, ReturnType<typeof setTimeout>>(),
    };
    lifetime.current = owned;
    void productSyncDatabase.legacyRecords().then((value) => {
      if (owned.active) setKeys(value);
    }).catch(() => {
      if (owned.active) setError(true);
    });
    const end = (): void => {
      owned.active = false;
      for (const [url, timer] of owned.urls) {
        clearTimeout(timer);
        URL.revokeObjectURL(url);
      }
      owned.urls.clear();
    };
    globalThis.addEventListener(PRODUCT_SESSION_END_EVENT, end);
    return () => {
      end();
      globalThis.removeEventListener(PRODUCT_SESSION_END_EVENT, end);
    };
  }, []);
  const download = (key: string): void => {
    const owned = lifetime.current;
    if (!owned?.active || owned.busy) return;
    owned.busy = true;
    setBusy(true);
    void productSyncDatabase.exportLegacy(key).then((body) => {
      if (!owned.active) return;
      const url = URL.createObjectURL(
        new Blob([body], { type: "application/json" }),
      );
      owned.urls.set(
        url,
        setTimeout(() => {
          if (owned.urls.delete(url)) URL.revokeObjectURL(url);
        }, 1000),
      );
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = "cowboy-unowned-outbox.json";
      anchor.click();
    }).catch(() => {
      if (owned.active) setError(true);
    }).finally(() => {
      owned.busy = false;
      if (owned.active) setBusy(false);
    });
  };
  if (!keys.length && !error) return null;
  return (
    <Alert severity="warning">
      <Stack spacing={1}>
        <Typography variant="body2">
          {error
            ? "Local outbox inspection failed. Existing browser data has not been deleted; do not repeat a send based on this error."
            : `${keys.length} legacy local records were retained separately. Their Service/account ownership is unknown, so they will not be loaded or sent automatically.`}
        </Typography>
        {!!keys.length && (
          <Button
            color="inherit"
            size="small"
            onClick={() => setExpanded((value) => !value)}
          >
            {expanded ? "Hide local recovery" : "Review local recovery"}
          </Button>
        )}
        {expanded && (
          <>
            <Typography variant="caption">
              Download one private record for review. It may contain prompts or
              attachments. This does not import it, repeat an operation, or
              delete the original.
            </Typography>
            {keys.slice(page * 32, (page + 1) * 32).map((key, index) => (
              <Button
                key={key}
                disabled={busy}
                size="small"
                onClick={() => download(key)}
              >
                Download retained record {page * 32 + index + 1}
              </Button>
            ))}
            {keys.length > 32 && (
              <Stack direction="row" spacing={1}>
                <Button
                  size="small"
                  disabled={busy || page === 0}
                  onClick={() => setPage((value) => value - 1)}
                >
                  Previous records
                </Button>
                <Button
                  size="small"
                  disabled={busy || (page + 1) * 32 >= keys.length}
                  onClick={() => setPage((value) => value + 1)}
                >
                  Next records
                </Button>
              </Stack>
            )}
          </>
        )}
      </Stack>
    </Alert>
  );
}
