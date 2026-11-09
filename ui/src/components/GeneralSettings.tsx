import { useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import { Monitor, Moon, Sun } from "lucide-react";
import { type ReactNode, useState } from "react";

import { api, errorMessage } from "../lib/api";
import { keys, useIsolationSettings } from "../lib/queries";
import { TERMINAL_FONT_SIZES, type Theme, useUi } from "../lib/store";
import { Checkbox, ErrorText, Field, Select } from "./ui";

const themes: { id: Theme; label: string; icon: ReactNode }[] = [
  { id: "system", label: "System", icon: <Monitor size={13} /> },
  { id: "light", label: "Light", icon: <Sun size={13} /> },
  { id: "dark", label: "Dark", icon: <Moon size={13} /> },
];

export function GeneralSettings() {
  const qc = useQueryClient();
  const theme = useUi((s) => s.theme);
  const setTheme = useUi((s) => s.setTheme);
  const fontSize = useUi((s) => s.terminalFontSize);
  const setFontSize = useUi((s) => s.setTerminalFontSize);
  const reopen = useUi((s) => s.reopenSessions);
  const setReopen = useUi((s) => s.setReopenSessions);
  const { data: isolation } = useIsolationSettings();
  const [error, setError] = useState<string | null>(null);

  async function setStopOnQuit(stopOnQuit: boolean) {
    if (!isolation) return;
    try {
      await api.saveIsolationSettings({ ...isolation, stopOnQuit });
      setError(null);
      await qc.invalidateQueries({ queryKey: keys.isolationSettings });
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <section className="flex min-h-0 flex-1 flex-col">
      <h2 className="px-6 pt-5 pb-3 text-[15px] font-semibold">General</h2>
      <div className="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto px-6 pb-6">
        <Group title="Appearance">
          <Field label="Theme">
            <div className="flex w-fit rounded-md border border-line p-0.5">
              {themes.map((t) => (
                <button
                  key={t.id}
                  onClick={() => setTheme(t.id)}
                  className={clsx(
                    "flex h-7 items-center gap-1.5 rounded px-3 text-[13px]",
                    theme === t.id ? "bg-selected font-medium text-fg" : "text-muted hover:text-fg",
                  )}
                >
                  {t.icon}
                  {t.label}
                </button>
              ))}
            </div>
          </Field>
          <Field label="Terminal font size">
            <Select
              value={fontSize}
              onChange={(e) => setFontSize(Number(e.target.value))}
              className="h-8 w-24 text-[13px]"
            >
              {TERMINAL_FONT_SIZES.map((size) => (
                <option key={size} value={size}>
                  {size} pt
                </option>
              ))}
            </Select>
          </Field>
        </Group>

        <Group title="Startup and quit">
          <Checkbox checked={reopen} onChange={setReopen}>
            <span className="text-[13px]">Reopen Claude sessions on start</span>
            <span className="block text-xs text-faint">
              Claude sessions that were open when worki quit start again and continue their conversations.
            </span>
          </Checkbox>
          <Checkbox checked={isolation?.stopOnQuit ?? false} onChange={setStopOnQuit}>
            <span className="text-[13px]">Stop containers when worki quits</span>
            <span className="block text-xs text-faint">
              Off: Docker, Podman and sandbox environments keep running, so sessions start faster next time. On: they're
              stopped to free memory and started again when a session needs them.
            </span>
          </Checkbox>
          <ErrorText>{error}</ErrorText>
        </Group>
      </div>
    </section>
  );
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-3">
      <h3 className="text-[11px] font-semibold tracking-wide text-faint uppercase">{title}</h3>
      {children}
    </div>
  );
}
