import { listen } from "@tauri-apps/api/event";
import { LoaderCircle } from "lucide-react";
import { useEffect, useState } from "react";

export function QuittingOverlay() {
  const [quitting, setQuitting] = useState(false);

  useEffect(() => {
    const unlisten = listen("app-quitting", () => setQuitting(true));
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  if (!quitting) return null;
  return (
    <div
      data-tauri-drag-region
      className="fixed inset-0 z-50 flex animate-[fade-in_200ms_ease-out_300ms_both] items-center justify-center bg-black/40 backdrop-blur-sm"
    >
      <div className="flex items-center gap-3 rounded-xl border border-line bg-elevated px-5 py-4 shadow-2xl">
        <LoaderCircle size={18} className="animate-spin text-accent" />
        <div>
          <p className="text-sm font-medium">Quitting worki…</p>
          <p className="text-xs text-muted">Ending sessions and stopping sandboxes and containers.</p>
        </div>
      </div>
    </div>
  );
}
