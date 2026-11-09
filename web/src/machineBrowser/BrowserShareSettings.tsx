/** Account settings: owner-defined shared browser profiles. Cards in one
 * group use one Machine browser profile, so a common identity (such as
 * Google) is signed in once. A group holds at most one account of each
 * family; a candidate whose family is present is offered disabled with the
 * reason. Every change goes through the journaled account writer (`save`).
 * See docs/machine-browser.md, "Shared browser profiles". */
import { Button, Chip, Paper, Stack, TextField, Typography } from "@mui/material";
import { useState } from "react";
import type {
  BrowserShare,
  BrowserShareMemberRef,
  SubscriptionSnapshot,
} from "../subscriptionAccounts";
import { validSubscriptionLabel } from "../subscriptionDisplay";
import { joinBlocked, memberLabel, shareCandidates, shareOf } from "./shareRules";

export type BrowserShareChange =
  | { type: "create_browser_share"; label: string }
  | { type: "rename_browser_share"; share_id: string; label: string }
  | { type: "delete_browser_share"; share_id: string }
  | { type: "set_browser_share"; member: BrowserShareMemberRef; share_id: string | null };

/** Send a change and clear its draft only once it committed: a revision
 * conflict or a refusal keeps the owner's text for another try. */
export async function commitDraft(
  save: (change: BrowserShareChange) => Promise<boolean>,
  change: BrowserShareChange,
  clear: () => void,
): Promise<boolean> {
  const committed = await save(change);
  if (committed) clear();
  return committed;
}

export function BrowserShareSettings({
  snapshot,
  disabled,
  save,
}: {
  snapshot: SubscriptionSnapshot | undefined;
  disabled: boolean;
  /** Resolves `true` once the change is committed. */
  save: (change: BrowserShareChange) => Promise<boolean>;
}): React.JSX.Element | null {
  const [label, setLabel] = useState("");
  if (!snapshot || snapshot.machine_browser === undefined) return null;
  const shares = snapshot.browser_shares ?? [];
  return (
    <Stack spacing={1} data-browser-shares>
      <Typography variant="subtitle2" color="text.primary">
        Shared browser
      </Typography>
      <Typography variant="caption" color="text.secondary">
        Cards in one group use one browser profile, so you sign in to a shared identity such as
        Google once. A group holds at most one account of each kind; each card still needs its own
        browser enabled.
      </Typography>
      {shares.map((share) => (
        <ShareGroup
          key={share.share_id}
          snapshot={snapshot}
          share={share}
          disabled={disabled}
          save={save}
        />
      ))}
      <Stack direction={{ xs: "column", sm: "row" }} spacing={1}>
        <TextField
          size="small"
          label="New group"
          placeholder="Google"
          value={label}
          disabled={disabled}
          fullWidth
          onChange={(event) => setLabel(event.target.value)}
        />
        <Button
          disabled={disabled || !validSubscriptionLabel(label)}
          onClick={() => {
            // The draft stays until the write is confirmed (a conflict or a
            // refusal keeps it for another try).
            void commitDraft(save, { type: "create_browser_share", label: label.trim() }, () =>
              setLabel(""),
            );
          }}
          sx={{ minHeight: 40, flexShrink: 0 }}
        >
          Create group
        </Button>
      </Stack>
    </Stack>
  );
}

function ShareGroup({
  snapshot,
  share,
  disabled,
  save,
}: {
  snapshot: SubscriptionSnapshot;
  share: BrowserShare;
  disabled: boolean;
  save: (change: BrowserShareChange) => Promise<boolean>;
}): React.JSX.Element {
  const [name, setName] = useState<string | undefined>(undefined);
  const [confirmDelete, setConfirmDelete] = useState(false);
  if (share.deleting) {
    return (
      <Paper variant="outlined" sx={{ p: 1.5 }} data-browser-share={share.share_id}>
        <Typography variant="body2">{share.label}</Typography>
        <Typography variant="caption" color="text.secondary">
          Deleting: its browser profile is being removed from its Machine.
        </Typography>
      </Paper>
    );
  }
  const draft = name ?? share.label;
  return (
    <Paper variant="outlined" sx={{ p: 1.5 }} data-browser-share={share.share_id}>
      <Stack spacing={1}>
        <Stack direction="row" spacing={1} alignItems="center">
          <TextField
            size="small"
            label="Group"
            value={draft}
            disabled={disabled}
            fullWidth
            sx={{ minWidth: 0 }}
            onChange={(event) => setName(event.target.value)}
          />
          <Button
            disabled={disabled || !validSubscriptionLabel(draft) || draft.trim() === share.label}
            onClick={() => {
              void commitDraft(
                save,
                { type: "rename_browser_share", share_id: share.share_id, label: draft.trim() },
                () => setName(undefined),
              );
            }}
          >
            Save
          </Button>
        </Stack>
        <Stack direction="row" spacing={0.5} flexWrap="wrap" useFlexGap>
          {share.members.length === 0 && (
            <Typography variant="caption" color="text.secondary">
              No cards yet.
            </Typography>
          )}
          {share.members.map((member) => (
            <Chip
              key={member.account_id}
              label={memberLabel(snapshot, member)}
              disabled={disabled}
              aria-label={`${memberLabel(snapshot, member)} in ${share.label}`}
              onDelete={() =>
                void save({ type: "set_browser_share", member: member.member, share_id: null })
              }
            />
          ))}
        </Stack>
        <ShareCandidates snapshot={snapshot} share={share} disabled={disabled} save={save} />
        {confirmDelete ? (
          <Stack spacing={0.5}>
            <Typography variant="caption" color="text.secondary">
              Deleting the group deletes its browser profile (its cookies and sign-ins) on its
              Machine. Each card goes back to its own browser.
            </Typography>
            <Stack direction="row" spacing={1}>
              <Button onClick={() => setConfirmDelete(false)}>Cancel</Button>
              <Button
                color="error"
                disabled={disabled}
                onClick={() => {
                  setConfirmDelete(false);
                  void save({ type: "delete_browser_share", share_id: share.share_id });
                }}
              >
                Delete group
              </Button>
            </Stack>
          </Stack>
        ) : (
          <Button
            color="error"
            disabled={disabled}
            onClick={() => setConfirmDelete(true)}
            sx={{ alignSelf: "flex-start" }}
          >
            Delete group…
          </Button>
        )}
      </Stack>
    </Paper>
  );
}

/** The cards that may join `share`, each with its contextual action and,
 * when it cannot join or would move, the reason. */
function ShareCandidates({
  snapshot,
  share,
  disabled,
  save,
}: {
  snapshot: SubscriptionSnapshot;
  share: BrowserShare;
  disabled: boolean;
  save: (change: BrowserShareChange) => Promise<boolean>;
}): React.JSX.Element | null {
  const members = new Set(share.members.map((member) => member.account_id));
  const candidates = shareCandidates(snapshot).filter(
    (candidate) => !members.has(candidate.browserId),
  );
  if (candidates.length === 0) return null;
  return (
    <Stack spacing={0.5}>
      <Typography variant="caption" color="text.secondary">
        Add a card
      </Typography>
      {candidates.map((candidate) => {
        const blocked = joinBlocked(share, candidate);
        const elsewhere = shareOf(snapshot, candidate.browserId);
        const reason = `share-${share.share_id}-reason-${candidate.browserId}`;
        return (
          <Stack
            key={candidate.browserId}
            direction="row"
            spacing={1}
            alignItems="center"
            data-share-candidate={candidate.browserId}
          >
            <Stack sx={{ flex: 1, minWidth: 0 }}>
              <Typography variant="body2" noWrap>
                {candidate.label}
              </Typography>
              {(blocked || elsewhere) && (
                <Typography variant="caption" color="text.secondary" id={reason}>
                  {blocked ?? `Moves it out of ${elsewhere!.label}.`}
                </Typography>
              )}
            </Stack>
            <Button
              size="small"
              disabled={disabled || blocked !== undefined}
              aria-label={`Add ${candidate.label} to ${share.label}`}
              aria-describedby={blocked || elsewhere ? reason : undefined}
              onClick={() =>
                void save({
                  type: "set_browser_share",
                  member: candidate.member,
                  share_id: share.share_id,
                })
              }
              sx={{ minHeight: 36, flexShrink: 0 }}
            >
              Add
            </Button>
          </Stack>
        );
      })}
    </Stack>
  );
}
