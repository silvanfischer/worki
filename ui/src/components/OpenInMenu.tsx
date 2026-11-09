import { AppWindow, ChevronDown, Settings2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { api, errorMessage, type OpenerApp } from "../lib/api";
import { useAppIcon, useOpenerSettings } from "../lib/queries";
import { useUi } from "../lib/store";

export function AppIcon({ app, size = 16 }: { app: OpenerApp; size?: number }) {
  const { data: icon } = useAppIcon(app.path);
  return icon ? (
    <img src={icon} alt="" width={size} height={size} className="shrink-0" draggable={false} />
  ) : (
    <AppWindow size={size - 2} className="shrink-0 text-muted" />
  );
}

/** Split button: opens the worktree in the default app; the arrow lists all configured apps. */
export function OpenInMenu({ path }: { path: string }) {
  const { data: settings } = useOpenerSettings();
  const openDialog = useUi((s) => s.openDialog);
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);

  const apps = settings?.apps ?? [];
  const primary = apps.find((a) => a.id === settings?.defaultApp) ?? apps[0];

  async function openWith(app: OpenerApp) {
    setOpen(false);
    setError(null);
    try {
      await api.openInApp(app.path, path);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <div ref={ref} className="relative">
      <div className="flex h-7 items-center rounded-md border border-line bg-elevated">
        <button
          onClick={() => (primary ? openWith(primary) : openDialog({ kind: "settings", tab: "openIn" }))}
          title={primary ? `Open in ${primary.name}` : "Choose apps to open worktrees in"}
          className="flex h-full items-center gap-1.5 rounded-l-md pr-1.5 pl-2 text-xs hover:bg-hover"
        >
          {primary ? <AppIcon app={primary} /> : <AppWindow size={14} className="text-muted" />}
          <span className="max-w-28 truncate">{primary ? primary.name : "Open in…"}</span>
        </button>
        <button
          onClick={() => setOpen((o) => !o)}
          aria-label="Choose app"
          className="flex h-full items-center rounded-r-md border-l border-line px-1 text-muted hover:bg-hover hover:text-fg"
        >
          <ChevronDown size={13} />
        </button>
      </div>
      {open && (
        <div className="absolute right-0 z-40 mt-1 min-w-52 rounded-lg border border-line bg-elevated p-1 shadow-xl">
          {apps.map((app) => (
            <button
              key={app.id}
              onClick={() => openWith(app)}
              className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] hover:bg-accent hover:text-accent-fg"
            >
              <AppIcon app={app} />
              {app.name}
            </button>
          ))}
          {apps.length > 0 && <div className="my-1 border-t border-line" />}
          <button
            onClick={() => {
              setOpen(false);
              openDialog({ kind: "settings", tab: "openIn" });
            }}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] text-muted hover:bg-accent hover:text-accent-fg"
          >
            <Settings2 size={14} />
            Manage apps…
          </button>
        </div>
      )}
      {error && (
        <div className="absolute right-0 z-40 mt-1 w-72 rounded-md border border-removed/30 bg-elevated p-2 text-xs text-removed shadow-xl">
          {error}
        </div>
      )}
    </div>
  );
}
