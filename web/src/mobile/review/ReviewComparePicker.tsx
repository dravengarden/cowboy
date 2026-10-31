import { Check } from "@mui/icons-material";
import {
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  ListSubheader,
  TextField,
} from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import { Sheet } from "../../Sheet";
import type { GitComparison } from "./codeApi";
import {
  reviewCompareBranches,
  type ReviewCompareScenario,
} from "./reviewCompareModel";

function Row({ primary, secondary, selected, onClick }: {
  primary: string;
  secondary?: string;
  selected: boolean;
  onClick: () => void;
}): React.JSX.Element {
  return (
    <ListItemButton
      selected={selected}
      aria-current={selected ? "true" : undefined}
      onClick={onClick}
    >
      <ListItemIcon sx={{ minWidth: 32 }}>
        {selected && <Check fontSize="small" color="primary" />}
      </ListItemIcon>
      <ListItemText
        primary={primary}
        {...(secondary ? { secondary } : {})}
        primaryTypographyProps={{ noWrap: true }}
      />
    </ListItemButton>
  );
}

/** Choose by the reason to look; the branch list is for everything else. */
export function ReviewComparePicker({
  open,
  onClose,
  comparison,
  scenarios,
  comparisons,
  onSelect,
}: {
  open: boolean;
  onClose: () => void;
  comparison: string | null | undefined;
  scenarios: readonly ReviewCompareScenario[];
  comparisons: readonly GitComparison[];
  onSelect: (comparison: string | null) => void;
}): React.JSX.Element {
  const [query, setQuery] = useState("");
  useEffect(() => {
    if (!open) setQuery("");
  }, [open]);
  const branches = useMemo(
    () => reviewCompareBranches(comparisons, scenarios, query),
    [comparisons, query, scenarios],
  );
  return (
    <Sheet
      open={open}
      onClose={onClose}
      title="Compare"
      forceSheet
      mobileDismiss="footer"
      portal
    >
      <List dense disablePadding aria-label="Comparison scenarios">
        {scenarios.map((scenario) => (
          <Row
            key={scenario.key}
            primary={scenario.title}
            secondary={scenario.detail}
            selected={scenario.comparison === comparison}
            onClick={() => onSelect(scenario.comparison)}
          />
        ))}
      </List>
      {comparisons.length > 0 && (
        <List
          dense
          disablePadding
          aria-label="Other branches"
          subheader={
            <ListSubheader disableSticky sx={{ bgcolor: "transparent" }}>
              Other branches
            </ListSubheader>
          }
        >
          <TextField
            size="small"
            fullWidth
            placeholder="Search branches"
            aria-label="Search branches"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            sx={{ px: 2, pb: 1 }}
          />
          {branches.map((branch) => (
            <Row
              key={branch.reference}
              primary={branch.label}
              selected={branch.reference === comparison}
              onClick={() => onSelect(branch.reference)}
            />
          ))}
        </List>
      )}
    </Sheet>
  );
}
