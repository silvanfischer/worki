import clsx from "clsx";
import { CircleAlert, Info, X } from "lucide-react";

import { useToasts } from "../lib/toast";

export function Toaster() {
  const toasts = useToasts((s) => s.toasts);
  const dismiss = useToasts((s) => s.dismiss);
  if (toasts.length === 0) return null;
  return (
    <div className="pointer-events-none fixed right-4 bottom-4 z-[60] flex w-80 flex-col gap-2">
      {toasts.map((t) => (
        <div
          key={t.id}
          role={t.kind === "error" ? "alert" : "status"}
          className="pointer-events-auto flex items-start gap-2 rounded-lg border border-line bg-elevated px-3 py-2.5 text-xs shadow-lg"
        >
          {t.kind === "error" ? (
            <CircleAlert size={14} className="mt-px shrink-0 text-removed" />
          ) : (
            <Info size={14} className="mt-px shrink-0 text-accent" />
          )}
          <span className={clsx("flex-1 leading-relaxed", t.kind === "error" ? "text-fg" : "text-muted")}>{t.message}</span>
          <button aria-label="Dismiss" onClick={() => dismiss(t.id)} className="text-faint hover:text-fg">
            <X size={12} />
          </button>
        </div>
      ))}
    </div>
  );
}
