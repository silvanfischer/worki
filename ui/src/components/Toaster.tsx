import clsx from "clsx";
import { CircleAlert, Copy, Info, X } from "lucide-react";

import { useToasts } from "../lib/toast";

export function Toaster() {
  const toasts = useToasts((s) => s.toasts);
  const dismiss = useToasts((s) => s.dismiss);
  const keep = useToasts((s) => s.keep);
  if (toasts.length === 0) return null;
  return (
    <div className="pointer-events-none fixed right-4 bottom-4 z-[60] flex w-80 flex-col gap-2">
      {toasts.map((t) => (
        <div
          key={t.id}
          role={t.kind === "error" ? "alert" : "status"}
          onMouseEnter={() => keep(t.id)}
          className="pointer-events-auto flex items-start gap-2 rounded-lg border border-line bg-elevated px-3 py-2.5 text-[13px] shadow-lg"
        >
          {t.kind === "error" ? (
            <CircleAlert size={15} className="mt-0.5 shrink-0 text-removed" />
          ) : (
            <Info size={15} className="mt-0.5 shrink-0 text-accent" />
          )}
          <span
            className={clsx(
              "selectable min-w-0 flex-1 leading-relaxed break-words",
              t.kind === "error" ? "text-fg" : "text-muted",
            )}
          >
            {t.message}
          </span>
          <button
            aria-label="Copy message"
            title="Copy"
            onClick={() => {
              navigator.clipboard.writeText(t.message);
              keep(t.id);
            }}
            className="text-faint hover:text-fg"
          >
            <Copy size={14} />
          </button>
          <button aria-label="Dismiss" onClick={() => dismiss(t.id)} className="text-faint hover:text-fg">
            <X size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
