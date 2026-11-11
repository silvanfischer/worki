import clsx from "clsx";
import { Monitor, Moon, Sun } from "lucide-react";
import type { ReactNode } from "react";

import { FONT_SIZES, type Theme, useUi } from "../lib/store";
import { Group } from "./GeneralSettings";
import { Field, Select } from "./ui";

const themes: { id: Theme; label: string; icon: ReactNode }[] = [
  { id: "system", label: "System", icon: <Monitor size={13} /> },
  { id: "light", label: "Light", icon: <Sun size={13} /> },
  { id: "dark", label: "Dark", icon: <Moon size={13} /> },
];

export function AppearanceSettings() {
  const ui = useUi();
  return (
    <section className="flex min-h-0 flex-1 flex-col">
      <h2 className="px-6 pt-5 pb-3 text-[15px] font-semibold">Appearance</h2>
      <div className="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto px-6 pb-6">
        <Group title="Theme">
          <div className="flex w-fit rounded-md border border-line p-0.5">
            {themes.map((t) => (
              <button
                key={t.id}
                onClick={() => ui.setTheme(t.id)}
                className={clsx(
                  "flex h-7 items-center gap-1.5 rounded px-3 text-[13px]",
                  ui.theme === t.id ? "bg-selected font-medium text-fg" : "text-muted hover:text-fg",
                )}
              >
                {t.icon}
                {t.label}
              </button>
            ))}
          </div>
        </Group>

        <Group title="Font sizes">
          <FontSizeField label="Sidebar" value={ui.sidebarFontSize} onChange={ui.setSidebarFontSize} />
          <FontSizeField label="Terminal" value={ui.terminalFontSize} onChange={ui.setTerminalFontSize} />
          <FontSizeField label="Chat" value={ui.chatFontSize} onChange={ui.setChatFontSize} />
          <FontSizeField label="Changes panel" value={ui.panelFontSize} onChange={ui.setPanelFontSize} />
          <FontSizeField label="Diff" value={ui.diffFontSize} onChange={ui.setDiffFontSize} />
        </Group>
      </div>
    </section>
  );
}

function FontSizeField({ label, value, onChange }: { label: string; value: number; onChange: (size: number) => void }) {
  return (
    <Field label={label}>
      <Select value={value} onChange={(e) => onChange(Number(e.target.value))} className="h-8 w-24 text-[13px]">
        {FONT_SIZES.map((size) => (
          <option key={size} value={size}>
            {size} pt
          </option>
        ))}
      </Select>
    </Field>
  );
}
