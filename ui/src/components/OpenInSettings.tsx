import { useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { Plus, Trash2 } from "lucide-react";
import { useState } from "react";

import { api, errorMessage, type OpenerSettings } from "../lib/api";
import { keys, useOpenerSettings } from "../lib/queries";
import { AppIcon } from "./OpenInMenu";
import { Button, ErrorText, IconButton } from "./ui";

/** Apps worktrees can be opened in. Changes are saved immediately. */
export function OpenInSettings() {
  const qc = useQueryClient();
  const { data: settings } = useOpenerSettings();
  const [error, setError] = useState<string | null>(null);

  async function save(next: OpenerSettings) {
    setError(null);
    try {
      await api.saveOpenerSettings(next);
      await qc.invalidateQueries({ queryKey: keys.openers });
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function addApp() {
    if (!settings) return;
    const path = await open({
      title: "Choose an app",
      defaultPath: "/Applications",
      filters: [{ name: "Applications", extensions: ["app"] }],
    });
    if (!path) return;
    try {
      const app = await api.openerAppFromPath(path);
      if (settings.apps.some((a) => a.path === app.path)) return;
      await save({ apps: [...settings.apps, app], defaultApp: settings.defaultApp ?? app.id });
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  if (!settings) return null;

  return (
    <section className="flex min-h-0 flex-1 flex-col">
      <h2 className="px-6 pt-5 pb-3 text-[15px] font-semibold">Open in</h2>
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-6 pb-6">
        <p className="text-xs leading-relaxed text-faint">
          Apps listed in the worktree header to open the worktree with. The default app is the main button. Apps run as
          you, not as the agent user. Saved right away.
        </p>
        <div className="flex flex-col gap-1">
          <div className="grid grid-cols-[52px_1fr_24px] gap-2 px-1 text-[11px] font-medium text-faint">
            <span>Default</span>
            <span>App</span>
          </div>
          {settings.apps.map((app) => (
            <div key={app.id} className="grid grid-cols-[52px_1fr_24px] items-center gap-2 rounded-md px-1 py-1 hover:bg-hover">
              <input
                type="radio"
                name="default-opener"
                className="justify-self-center accent-[var(--accent)]"
                checked={settings.defaultApp === app.id}
                onChange={() => save({ ...settings, defaultApp: app.id })}
              />
              <div className="flex min-w-0 items-center gap-2">
                <AppIcon app={app} size={20} />
                <span className="text-[13px]">{app.name}</span>
                <span className="truncate font-mono text-[11px] text-faint">{app.path}</span>
              </div>
              <IconButton
                label={`Remove ${app.name}`}
                onClick={() => {
                  const apps = settings.apps.filter((a) => a.id !== app.id);
                  const defaultApp = settings.defaultApp === app.id ? (apps[0]?.id ?? null) : settings.defaultApp;
                  save({ apps, defaultApp });
                }}
              >
                <Trash2 size={13} />
              </IconButton>
            </div>
          ))}
          {settings.apps.length === 0 && <p className="px-1 py-2 text-xs text-faint">No apps yet.</p>}
        </div>
        <Button className="self-start" onClick={addApp}>
          <Plus size={13} />
          Add app…
        </Button>
        <ErrorText>{error}</ErrorText>
      </div>
    </section>
  );
}
