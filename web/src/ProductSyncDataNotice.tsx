import { Alert, Button, Stack, TextField, Typography } from "@mui/material";
import { useEffect, useId, useRef, useState } from "react";
import { productSyncDatabase } from "./productSyncDatabase.ts";
import { PRODUCT_SESSION_END_EVENT } from "./productSessionEnd.ts";

/** An explicit local export, not a migration grant. Never sends a saved draft. */
export function ProductSyncDataNotice(): React.JSX.Element | null {
  const [keys, setKeys] = useState<string[]>([]);
  const [inspectionFailed, setInspectionFailed] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [busy, setBusy] = useState(false);
  const [selected, setSelected] = useState(0);
  const [result, setResult] = useState<
    { kind: "info" | "warning"; text: string } | null
  >(null);
  const detailsId = useId();
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
      if (owned.active) setInspectionFailed(true);
    });
    const stop = (): void => {
      owned.active = false;
      for (const [url, timer] of owned.urls) {
        clearTimeout(timer);
        URL.revokeObjectURL(url);
      }
      owned.urls.clear();
    };
    const end = (): void => {
      stop();
      setKeys([]);
      setInspectionFailed(false);
      setExpanded(false);
      setBusy(false);
      setResult(null);
    };
    globalThis.addEventListener(PRODUCT_SESSION_END_EVENT, end);
    return () => {
      stop();
      globalThis.removeEventListener(PRODUCT_SESSION_END_EVENT, end);
    };
  }, []);
  const download = (): void => {
    const owned = lifetime.current;
    const key = keys[selected];
    if (!owned?.active || owned.busy || key === undefined) return;
    owned.busy = true;
    setBusy(true);
    setResult(null);
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
      anchor.download = `cowboy-retained-record-${selected + 1}.json`;
      anchor.click();
      setResult({
        kind: "info",
        text: `Download requested for record ${
          selected + 1
        }. The original is unchanged.`,
      });
    }).catch(() => {
      if (owned.active) {
        setResult({
          kind: "warning",
          text:
            "This record could not be downloaded safely. It has not been deleted or sent. You can select another record.",
        });
      }
    }).finally(() => {
      owned.busy = false;
      if (owned.active) setBusy(false);
    });
  };
  const select = (index: number): void => {
    if (
      !lifetime.current?.active || lifetime.current.busy ||
      !Number.isSafeInteger(index) || index < 0 || index >= keys.length
    ) return;
    setSelected(index);
    setResult(null);
  };
  if (!keys.length && !inspectionFailed) return null;
  return (
    <Stack spacing={1} data-product-sync-recovery="local">
      <Typography variant="overline" color="text.secondary">
        Older browser data
      </Typography>
      {inspectionFailed && (
        <Alert severity="warning">
          Could not inspect older browser data. This does not mean it is empty.
          Nothing was deleted or sent; do not repeat a send based on this error.
        </Alert>
      )}
      {!!keys.length && (
        <>
          <Typography variant="body2">
            {keys.length} older{" "}
            {keys.length === 1 ? "record is" : "records are"}{" "}
            kept on this device. Their account cannot be verified, so they are
            not loaded or sent automatically.
          </Typography>
          <Button
            color="inherit"
            size="small"
            aria-expanded={expanded}
            aria-controls={detailsId}
            onClick={() => setExpanded((value) => !value)}
            sx={{ alignSelf: "flex-start", textTransform: "none" }}
          >
            {expanded ? "Hide older records" : "Review older records"}
          </Button>
          {expanded && (
            <Stack id={detailsId} spacing={1}>
              <Typography variant="body2" color="text.secondary">
                These are saved browser states, not telemetry logs. Download one
                record at a time for private review; it may contain prompts or
                attachments. Downloading does not restore, resend or delete it.
              </Typography>
              <TextField
                select
                fullWidth
                size="small"
                label="Retained record"
                value={selected}
                disabled={busy}
                slotProps={{ select: { native: true } }}
                onChange={(event) => select(Number(event.target.value))}
              >
                {keys.map((key, index) => (
                  <option key={key} value={index}>
                    Record {index + 1} of {keys.length}
                  </option>
                ))}
              </TextField>
              <Stack direction="row" justifyContent="space-between" spacing={1}>
                <Button
                  size="small"
                  disabled={busy || selected === 0}
                  onClick={() => select(selected - 1)}
                  sx={{ textTransform: "none" }}
                >
                  Previous
                </Button>
                <Button
                  size="small"
                  disabled={busy || selected === keys.length - 1}
                  onClick={() => select(selected + 1)}
                  sx={{ textTransform: "none" }}
                >
                  Next
                </Button>
              </Stack>
              <Button
                disabled={busy}
                size="small"
                variant="outlined"
                onClick={download}
                sx={{ textTransform: "none" }}
              >
                {busy ? "Preparing download…" : "Download selected record"}
              </Button>
              {result && (
                <Alert severity={result.kind} role="status">
                  {result.text}
                </Alert>
              )}
            </Stack>
          )}
        </>
      )}
    </Stack>
  );
}
