import { useMemo, useRef, useState } from "react";
import { Alert, Box, Button, IconButton, Menu, MenuItem, Stack, Tooltip, Typography } from "@mui/material";
import { DragIndicator, MoreVert } from "@mui/icons-material";
import { useSortable } from "./useSortable";
import { type AboutValues, type OrderChoice, type OrderKey, orderedItems } from "./aboutPreferences";

function OrderList({ title, choices, order, disabled, onChange }: {
  title: string; choices: OrderChoice[]; order: string[]; disabled: boolean; onChange: (ids: string[]) => void;
}): React.JSX.Element {
  const ids = useMemo(() => orderedItems(choices, order, (item) => item.id).map((item) => item.id), [choices, order]);
  const dragChange = useRef(onChange);
  const sortable = useSortable({ ids, onDragStart: () => { dragChange.current = onChange; }, onReorder: (next) => { if (!disabled) dragChange.current(next); }, optimisticReorder: false });
  const [menu, setMenu] = useState<{ anchor: HTMLElement; id: string }>();
  const move = (target: number) => {
    if (!menu) return;
    const next = ids.filter((id) => id !== menu.id);
    next.splice(target, 0, menu.id);
    setMenu(undefined);
    onChange(next);
  };
  const index = menu ? ids.indexOf(menu.id) : -1;
  return (
    <Stack spacing={0.5}>
      <Typography variant="overline" color="text.secondary">{title}</Typography>
      {!ids.length && <Typography variant="body2" color="text.secondary">No items yet.</Typography>}
      {sortable.order.map((id) => (
        <Stack key={id} ref={sortable.registerItem(id)} style={sortable.itemStyle(id)} direction="row" alignItems="center"
          sx={{ border: 1, borderColor: "divider", borderRadius: 1.5, minHeight: 44, bgcolor: "background.paper" }}>
          <Tooltip title="Drag to reorder">
            <IconButton {...sortable.handleProps(id)} disabled={disabled} aria-label={`Drag ${choices.find((item) => item.id === id)?.name ?? id}`} tabIndex={-1} sx={{ width: 44, height: 44 }}><DragIndicator /></IconButton>
          </Tooltip>
          <Typography variant="body2" sx={{ flex: 1, minWidth: 0, overflowWrap: "anywhere" }}>{choices.find((item) => item.id === id)?.name ?? id}</Typography>
          <IconButton disabled={disabled} aria-label={`Move ${choices.find((item) => item.id === id)?.name ?? id}`} onClick={(event) => setMenu({ anchor: event.currentTarget, id })} sx={{ width: 44, height: 44 }}><MoreVert /></IconButton>
        </Stack>
      ))}
      <Menu anchorEl={menu?.anchor} open={!!menu} onClose={() => setMenu(undefined)}>
        <MenuItem disabled={disabled || index <= 0} onClick={() => move(0)}>Move to top</MenuItem>
        <MenuItem disabled={disabled || index <= 0} onClick={() => move(index - 1)}>Move up</MenuItem>
        <MenuItem disabled={disabled || index >= ids.length - 1} onClick={() => move(index + 1)}>Move down</MenuItem>
        <MenuItem disabled={disabled || index >= ids.length - 1} onClick={() => move(ids.length - 1)}>Move to bottom</MenuItem>
      </Menu>
    </Stack>
  );
}

export function AboutLayoutEditor({ choices, values, disabled, onChange, desktop }: {
  choices: Record<OrderKey, OrderChoice[]>; values: AboutValues; disabled: boolean;
  onChange: (changes: Partial<AboutValues>) => void; desktop: boolean;
}): React.JSX.Element {
  return (
    <Stack spacing={2} data-about-layout-editor>
      <Typography variant="body2" color="text.secondary">Drag the handles or use each row’s Move menu. Saved to your account across devices.</Typography>
      <Box sx={{ display: "grid", gridTemplateColumns: desktop ? "repeat(2, minmax(0, 1fr))" : "minmax(0, 1fr)", gap: 2 }}>
        <OrderList title="Usage" choices={choices.usage_order} order={values.usage_order} disabled={disabled} onChange={(usage_order) => onChange({ usage_order })} />
        <OrderList title="Machines" choices={choices.machine_order} order={values.machine_order} disabled={disabled} onChange={(machine_order) => onChange({ machine_order })} />
      </Box>
      <Box><Button disabled={disabled} onClick={() => onChange({ usage_order: [], machine_order: [] })}>Restore default order</Button></Box>
      {disabled && <Alert severity="info">Finish saving or resolve the pending change before reordering again.</Alert>}
    </Stack>
  );
}
