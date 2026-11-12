import clsx from "clsx";
import { useRef } from "react";

import { type Panel, PANEL_LIMITS, useUi } from "../lib/store";

/**
 * Drag handle on one edge of a panel. `edge` is the side the handle sits on: dragging a right
 * edge to the right grows the panel, dragging a left or top edge away from the panel grows it.
 */
export function ResizeHandle({ panel, edge }: { panel: Panel; edge: "left" | "right" | "top" }) {
  const size = useUi((s) => s.panelWidths[panel]);
  const setSize = useUi((s) => s.setPanelWidth);
  const start = useRef<{ at: number; size: number } | null>(null);
  const vertical = edge === "top";
  const position = (e: React.PointerEvent) => (vertical ? e.clientY : e.clientX);

  return (
    <div
      role="separator"
      aria-orientation={vertical ? "horizontal" : "vertical"}
      title="Drag to resize, double-click to reset"
      onPointerDown={(e) => {
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        start.current = { at: position(e), size };
        document.body.style.cursor = vertical ? "row-resize" : "col-resize";
      }}
      onPointerMove={(e) => {
        if (!start.current) return;
        const moved = position(e) - start.current.at;
        setSize(panel, start.current.size + (edge === "right" ? moved : -moved));
      }}
      onPointerUp={(e) => {
        start.current = null;
        e.currentTarget.releasePointerCapture(e.pointerId);
        document.body.style.cursor = "";
      }}
      onDoubleClick={() => setSize(panel, PANEL_LIMITS[panel].initial)}
      className={clsx(
        "group absolute z-20",
        vertical
          ? "inset-x-0 -top-1 flex h-2 cursor-row-resize items-center"
          : ["top-0 bottom-0 w-2 cursor-col-resize", edge === "right" ? "-right-1" : "-left-1"],
      )}
    >
      <div
        className={clsx(
          "bg-transparent transition-colors group-hover:bg-accent group-active:bg-accent",
          vertical ? "h-px w-full" : "mx-auto h-full w-px",
        )}
      />
    </div>
  );
}
