// Fenced blocks that render as something richer than code while keeping the
// code block as their fallback: Mermaid diagrams and Cowboy widgets.

import type { ReactNode } from "react";
import { WIDGET_FENCE } from "../../contracts/widget-v1.ts";
import { MermaidDiagram } from "./MermaidDiagram";
import { WidgetFence } from "./widgets/WidgetFence";

export function richFence(
  lang: string,
  source: string,
  pending: boolean,
  block: ReactNode,
): ReactNode {
  switch (lang.toLowerCase()) {
    case "mermaid":
      return <MermaidDiagram source={source} pending={pending} fallback={block} />;
    case WIDGET_FENCE:
      return <WidgetFence source={source} pending={pending} fallback={block} />;
    default:
      return block;
  }
}
