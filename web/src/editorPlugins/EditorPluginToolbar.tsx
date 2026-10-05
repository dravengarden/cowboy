import {
  AccessTime,
  AutoFixHigh,
  Calculate,
  FormatListBulleted,
  LabelOutlined,
  LinkOutlined,
  SortByAlpha,
  TextFields,
} from "@mui/icons-material";
import { Box, Button, Tooltip } from "@mui/material";
import type { ReactNode } from "react";
import { useDesktopCommands } from "../desktop/commands/DesktopCommandProvider";
import { editorPluginCommandId } from "./appHost";
import { useEditorPlugins } from "./EditorPluginManager";
import type { EditorPluginIcon } from "./manifest";

const ICONS: Record<EditorPluginIcon, ReactNode> = {
  text: <TextFields />,
  sort: <SortByAlpha />,
  list: <FormatListBulleted />,
  wand: <AutoFixHigh />,
  clock: <AccessTime />,
  calc: <Calculate />,
  tag: <LabelOutlined />,
  link: <LinkOutlined />,
};

/** Desktop toolbar buttons that plugins contributed with `toolbar: true`.
 * Each button executes the same registered command as the Command Palette,
 * and keeps keyboard focus in the editor it acts on. */
export function EditorPluginToolbar(
  { kind, disabled }: { kind: "document" | "session"; disabled: boolean },
): React.JSX.Element | null {
  const { plugins } = useEditorPlugins();
  const { execute } = useDesktopCommands();
  const buttons = plugins.flatMap((plugin) =>
    plugin.manifest.contexts.includes(kind) &&
      plugin.manifest.surfaces.includes("desktop")
      ? plugin.commands.filter((c) => c.toolbar).map((command) => ({
        plugin,
        command,
      }))
      : []
  );
  if (buttons.length === 0) return null;
  return (
    <Box
      role="group"
      aria-label="Plugin tools"
      data-editor-plugin-toolbar
      sx={{ display: "contents" }}
    >
      {buttons.map(({ plugin, command }) => {
        const label = `${plugin.manifest.name}: ${command.title}`;
        return (
          <Tooltip key={`${plugin.manifest.id}:${command.id}`} title={label}>
            <span>
              <Button
                size="small"
                color="inherit"
                aria-label={label}
                data-editor-plugin-command={`${plugin.manifest.id}:${command.id}`}
                disabled={disabled}
                onPointerDown={(e) => {
                  if (e.button === 0) {
                    e.preventDefault();
                  }
                }}
                onClick={() =>
                  execute(
                    editorPluginCommandId(plugin.manifest.id, command.id),
                  )}
                sx={{
                  minWidth: 0,
                  minHeight: "2.25rem",
                  px: "0.5rem",
                  "& .MuiSvgIcon-root": { fontSize: "1.25rem" },
                }}
              >
                {command.icon
                  ? ICONS[command.icon]
                  : (
                    <Box component="span" sx={{ fontSize: "0.8125rem" }}>
                      {command.title.slice(0, 2)}
                    </Box>
                  )}
              </Button>
            </span>
          </Tooltip>
        );
      })}
    </Box>
  );
}
