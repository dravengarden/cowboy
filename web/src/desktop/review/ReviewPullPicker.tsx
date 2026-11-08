import {
  Alert,
  Box,
  Button,
  InputBase,
  List,
  ListItemButton,
  ListItemText,
  MenuItem,
  Select,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
} from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { desktopSize } from "../../surface/desktopSize";
import { DESKTOP_SURFACE_RADIUS } from "../DesktopEmbeddedControl";
import { extensionRequest, type PullPage, type PullSummary, resourceQuery } from "../../extensions/api";
import { pullNumber } from "../../review/remoteReviewModel";
import type { Choice } from "../../review/remoteReviewSources";

const RELATIONS = [
  { value: "author", label: "Mine" },
  { value: "review", label: "Review requested" },
  { value: "assigned", label: "Assigned" },
  { value: "involved", label: "Involved" },
] as const;

function discovers(choice: Choice | undefined): boolean {
  return !!choice?.extension.views.some((view) => view.id === choice.view && view.discovery);
}

/** The choice a typed number or URL opens: a URL names its own repository
 *  when the view can discover across repositories. */
export function choiceForInput(choice: Choice, input: string): { choice: Choice; number: string } {
  let target = choice;
  const text = input.trim();
  if (text.startsWith("https://") && discovers(choice)) {
    const parts = new URL(text).pathname.split("/");
    target = { ...choice, remote: { ...choice.remote, owner: parts[1] ?? "", repository: parts[2] ?? "" } };
  }
  return { choice: target, number: pullNumber(text, target.remote) };
}

/**
 * Choose a pull request: the account's PRs by relationship and state, or a
 * number or URL. Selecting one binds it to the Session's Review.
 */
export function ReviewPullPicker({
  context,
  available,
  onSelect,
}: {
  context: string;
  available: readonly Choice[];
  onSelect: (choice: Choice, number: string) => Promise<void>;
}): React.JSX.Element {
  const [selection, setSelection] = useState(available[0]?.key ?? "");
  const choice = available.find((item) => item.key === selection) ?? available[0];
  const [relation, setRelation] = useState("author");
  const [state, setState] = useState("open");
  const [page, setPage] = useState<PullPage>();
  const [error, setError] = useState<string>();
  const [input, setInput] = useState("");
  const [opening, setOpening] = useState(false);
  const pending = useRef<AbortController | undefined>(undefined);
  useEffect(() => {
    if (!choice || !discovers(choice)) return undefined;
    const controller = new AbortController();
    pending.current = controller;
    setPage(undefined);
    setError(undefined);
    const query = resourceQuery(choice.extension.identity, choice.remote.name, choice.view, "", 1);
    query.set("discovery", "true");
    query.set("relation", relation);
    query.set("state", state);
    query.set("currentRepository", "false");
    void extensionRequest(context, query, controller.signal).then((response) => {
      if (controller.signal.aborted) return;
      if (response.type !== "pulls") throw new Error("Update this Machine to browse pull requests.");
      setPage(response);
    }).catch((reason: unknown) => {
      if (!controller.signal.aborted) setError(reason instanceof Error ? reason.message : "Could not list pull requests.");
    });
    return () => controller.abort();
  }, [choice, context, relation, state]);
  const open = (target: Choice, number: string): void => {
    setOpening(true);
    setError(undefined);
    void onSelect(target, number).catch((reason: unknown) => {
      setError(reason instanceof Error ? reason.message : "Could not open the pull request.");
    }).finally(() => setOpening(false));
  };
  const pick = (item: PullSummary): void => {
    if (!choice) return;
    const [owner, repository] = item.repository.split("/");
    open({ ...choice, remote: { ...choice.remote, owner: owner!, repository: repository! } }, item.number);
  };
  return (
    <Box sx={{ display: "flex", flexDirection: "column", minHeight: 0, flex: 1 }} data-review-pull-picker>
      <Box sx={{ display: "flex", flexWrap: "wrap", gap: 0.5, px: 1, py: 0.5 }}>
        {available.length > 1 && (
          <Select size="small" value={choice?.key ?? ""} onChange={(event) => setSelection(event.target.value)} sx={{ fontSize: "0.75rem", minWidth: 0, flex: 1 }}>
            {available.map((item) => (
              <MenuItem key={item.key} value={item.key}>{`${item.remote.owner}/${item.remote.repository}`}</MenuItem>
            ))}
          </Select>
        )}
        <ToggleButtonGroup
          exclusive
          size="small"
          value={relation}
          onChange={(_event, value: string | null) => value && setRelation(value)}
          aria-label="Relationship"
          sx={{ "& .MuiToggleButton-root": { py: 0, px: 0.75, textTransform: "none", fontSize: "0.7rem", height: desktopSize(26) } }}
        >
          {RELATIONS.map((item) => <ToggleButton key={item.value} value={item.value}>{item.label}</ToggleButton>)}
        </ToggleButtonGroup>
        <Select size="small" value={state} onChange={(event) => setState(event.target.value)} aria-label="State" sx={{ fontSize: "0.72rem", height: desktopSize(26) }}>
          {["open", "merged", "closed", "all"].map((value) => <MenuItem key={value} value={value}>{value}</MenuItem>)}
        </Select>
      </Box>
      <Box component="form" sx={{ display: "flex", gap: 0.5, px: 1, pb: 0.5 }} onSubmit={(event) => {
        event.preventDefault();
        if (!choice) return;
        try {
          const target = choiceForInput(choice, input);
          open(target.choice, target.number);
        } catch {
          setError("Enter a PR number for this repository, or a full PR URL on this host.");
        }
      }}>
        <InputBase
          value={input}
          onChange={(event) => setInput(event.target.value)}
          placeholder="PR number or URL"
          inputProps={{ "aria-label": "Pull request number or URL", "data-review-pull-input": "" }}
          sx={{ flex: 1, px: 1, fontSize: "0.78rem", borderRadius: 1, bgcolor: "action.hover" }}
        />
        <Button type="submit" size="small" disabled={opening || !input.trim()}>Open</Button>
      </Box>
      {error && <Alert severity="warning" sx={{ mx: 1, py: 0 }}>{error}</Alert>}
      <List dense disablePadding sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: 0.5 }}>
        {!discovers(choice) && (
          <Typography sx={{ p: 2, fontSize: "0.8rem", color: "text.secondary" }}>
            This Machine's extension lists no pull requests; open one by number or URL.
          </Typography>
        )}
        {page?.items.length === 0 && (
          <Typography sx={{ p: 2, fontSize: "0.8rem", color: "text.secondary" }}>No pull requests match.</Typography>
        )}
        {page?.items.map((item) => (
          <ListItemButton
            key={item.url}
            data-desktop-item={item.url}
            data-review-pull={`${item.repository}#${item.number}`}
            disabled={opening}
            onClick={() => pick(item)}
            sx={{ py: 0.25, px: 1, minHeight: desktopSize(30), borderRadius: `${String(DESKTOP_SURFACE_RADIUS)}px` }}
          >
            <ListItemText
              primary={`${item.draft ? "Draft · " : ""}${item.title}`}
              secondary={`${item.repository}#${item.number} · ${item.author} · ${item.state}`}
              slotProps={{ primary: { noWrap: true, sx: { fontSize: "0.8rem" } }, secondary: { noWrap: true, sx: { fontSize: "0.68rem" } } }}
            />
          </ListItemButton>
        ))}
      </List>
    </Box>
  );
}
