import { Check, Clear } from "@mui/icons-material";
import {
  IconButton,
  InputAdornment,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  ListSubheader,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import { Sheet } from "../../Sheet";
import type { GitComparison } from "../../review/codeApi";
import {
  reviewCompareBranches,
  type ReviewCompareScenario,
} from "../../review/reviewCompareModel";

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
      sx={{ minHeight: 44 }}
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
            slotProps={{
              input: {
                sx: { fontSize: "max(16px, 1rem)" },
                endAdornment: query
                  ? (
                    <InputAdornment position="end">
                      <IconButton
                        aria-label="Clear branch search"
                        onPointerDown={(event) => event.preventDefault()}
                        onClick={() => setQuery("")}
                        sx={{ width: 44, height: 44, mr: -1 }}
                      >
                        <Clear fontSize="small" />
                      </IconButton>
                    </InputAdornment>
                  )
                  : undefined,
              },
            }}
            sx={{ px: 2, pb: 1 }}
          />
          {branches.length === 0 && query.trim().length > 0 && (
            <Typography
              variant="body2"
              color="text.secondary"
              sx={{ px: 2, py: 2, textAlign: "center" }}
            >
              No matching branches
            </Typography>
          )}
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
