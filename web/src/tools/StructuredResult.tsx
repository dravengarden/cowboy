import { Box, Stack, Typography } from "@mui/material";
import { Markdown } from "../Markdown";
import { CodeView, KeyValues, Labeled } from "./blocks";
import { Collapsible } from "./Collapsible";
import { record } from "./result";

/** Small semantic views for MCP data. Unknown shapes remain complete,
 * selectable JSON, so a new server/provider needs no host release to work. */
export function StructuredResult(
  { value, memory = false }: { value: unknown; memory?: boolean },
): React.JSX.Element {
  const object = record(value);
  if (!object) {
    return (
      <CodeView
        code={JSON.stringify(value, null, 2) ?? String(value)}
        lang="json"
        maxHeight={360}
        touchWrap
      />
    );
  }
  const { types, hits, ...rest } = object;
  if (
    memory && typeof object.title === "string" && typeof object.body === "string" &&
    typeof object.id === "string" && typeof object.revision === "number"
  ) {
    const { title, body, id, revision, ...metadata } = object;
    return (
      <Stack spacing={0.75} data-memory-record>
        <Typography variant="body2" sx={{ fontWeight: 650 }}>
          {title}
        </Typography>
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{
            overflowWrap: "anywhere",
            fontFamily: "ui-monospace, monospace",
          }}
        >
          {id} · revision {revision}
        </Typography>
        <Collapsible maxHeight={420}>
          <Markdown text={body} />
        </Collapsible>
        <ResponseDetails data={metadata} />
      </Stack>
    );
  }
  if (typeof types === "string") {
    return (
      <Stack spacing={1}>
        {Object.keys(rest).length > 0 && <KeyValues data={rest} />}
        <Labeled label="API reference">
          <CodeView code={types} lang="typescript" maxHeight={420} touchWrap />
        </Labeled>
        {hits !== undefined && <KeyValues data={{ hits }} />}
      </Stack>
    );
  }
  if (
    memory && Array.isArray(hits) &&
    hits.every((hit) => typeof record(hit)?.title === "string")
  ) {
    return (
      <Stack spacing={1} data-memory-results>
        <Typography variant="caption" color="text.secondary">
          {hits.length === 0
            ? "No matching memories"
            : `${hits.length} ${hits.length === 1 ? "memory" : "memories"}`}
          {object.truncated === true ? " · more results available" : ""}
        </Typography>
        <Collapsible maxHeight={420}>
          <Stack spacing={1}>
            {hits.map((value, index) => {
              const hit = record(value)!;
              const { title, snippet, body, ref, ...metadata } = hit;
              const reference = record(ref);
              const text = typeof snippet === "string"
                ? snippet
                : typeof body === "string"
                ? body
                : "";
              return (
                <Box
                  key={index}
                  sx={{
                    p: 1.25,
                    bgcolor: "action.hover",
                    borderRadius: 1,
                    minWidth: 0,
                    overflowWrap: "anywhere",
                  }}
                >
                  <Typography variant="body2" sx={{ fontWeight: 650 }}>
                    {String(title)}
                  </Typography>
                  {text && <Markdown text={text} />}
                  {reference && (
                    <Typography
                      variant="caption"
                      color="text.secondary"
                      sx={{
                        display: "block",
                        fontFamily: "ui-monospace, monospace",
                        overflowWrap: "anywhere",
                      }}
                    >
                      {typeof reference.id === "string" ? reference.id : ""}
                      {typeof reference.revision === "number"
                        ? ` · revision ${reference.revision}`
                        : ""}
                    </Typography>
                  )}
                  {Object.keys(metadata).length > 0 && (
                    <KeyValues data={metadata} />
                  )}
                </Box>
              );
            })}
          </Stack>
        </Collapsible>
        <ResponseDetails data={rest} />
      </Stack>
    );
  }
  if (Object.keys(object).length === 0) {
    return <CodeView code="{}" lang="json" touchWrap />;
  }
  return <KeyValues data={object} />;
}

function ResponseDetails(
  { data }: { data: Record<string, unknown> },
): React.JSX.Element | null {
  if (Object.keys(data).length === 0) return null;
  return (
    <Box component="details">
      <Box
        component="summary"
        sx={{
          minHeight: 36,
          py: 0.75,
          cursor: "pointer",
          color: "text.secondary",
          fontSize: "0.75rem",
          "@media (pointer: coarse)": { minHeight: 44 },
        }}
      >
        Response details
      </Box>
      <KeyValues data={data} />
    </Box>
  );
}
