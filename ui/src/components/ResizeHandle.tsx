import clsx from "clsx";
import { useRef } from "react";

import { type Panel, PANEL_LIMITS, useUi } from "../lib/store";

/**
 * Drag handle on one edge of a panel. `edge` is the side the handle sits on: dragging a right
 * edge to the right grows the panel, dragging a left edge to the left grows it.
 */
export function ResizeHandle({ panel, edge }: { panel: Panel; edge: "left" | "right" }) {
  const width = useUi((s) => s.panelWidths[panel]);
  const setWidth = useUi((s) => s.setPanelWidth);
  const start = useRef<{ x: number; width: number } | null>(null);

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      title="Drag to resize, double-click to reset"
      onPointerDown={(e) => {
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        start.current = { x: e.clientX, width };
        document.body.style.cursor = "col-resize";
      }}
      onPointerMove={(e) => {
        if (!start.current) return;
        const dx = e.clientX - start.current.x;
        setWidth(panel, start.current.width + (edge === "right" ? dx : -dx));
      }}
      onPointerUp={(e) => {
        start.current = null;
        e.currentTarget.releasePointerCapture(e.pointerId);
        document.body.style.cursor = "";
      }}
      onDoubleClick={() => setWidth(panel, PANEL_LIMITS[panel].initial)}
      className={clsx(
        "group absolute top-0 bottom-0 z-20 w-2 cursor-col-resize",
        edge === "right" ? "-right-1" : "-left-1",
      )}
    >
      <div className="mx-auto h-full w-px bg-transparent transition-colors group-hover:bg-accent group-active:bg-accent" />
    </div>
  );
}
