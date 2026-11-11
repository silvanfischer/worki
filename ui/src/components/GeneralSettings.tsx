import { useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import { LoaderCircle, MessageSquare, RefreshCw, SquareTerminal } from "lucide-react";
import { type ReactNode, useState } from "react";

import { api, errorMessage } from "../lib/api";
import { keys, useIsolationSettings } from "../lib/queries";
import { useUi } from "../lib/store";
import { checkForUpdates, useUpdates } from "../lib/updates";
import { Button, Checkbox, ErrorText, Field } from "./ui";

export function GeneralSettings() {
  const qc = useQueryClient();
  const agentView = useUi((s) => s.agentView);
  const setAgentView = useUi((s) => s.setAgentView);
  const autoUpdate = useUi((s) => s.autoUpdate);
  const setAutoUpdate = useUi((s) => s.setAutoUpdate);
  const updates = useUpdates();
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
        <Group title="Agents">
          <Field
            label="Open agents as"
            hint="⌘T and the start buttons use this. The + menu offers both. Claude Code and Codex (experimental) can run as a chat; other agents always open in a terminal."
          >
            <div className="flex w-fit rounded-md border border-line p-0.5">
              {(
                [
                  ["terminal", "Terminal", <SquareTerminal key="t" size={13} />],
                  ["chat", "Chat", <MessageSquare key="c" size={13} />],
                ] as const
              ).map(([id, label, icon]) => (
                <button
                  key={id}
                  onClick={() => setAgentView(id)}
                  className={clsx(
                    "flex h-7 items-center gap-1.5 rounded px-3 text-[13px]",
                    agentView === id ? "bg-selected font-medium text-fg" : "text-muted hover:text-fg",
                  )}
                >
                  {icon}
                  {label}
                </button>
              ))}
            </div>
          </Field>
        </Group>

        <Group title="Updates">
          <Checkbox checked={autoUpdate} onChange={setAutoUpdate}>
            <span className="text-[13px]">Check for updates automatically</span>
            <span className="block text-xs text-faint">
              Looks for a new worki release on GitHub when worki starts and every few hours. A new release shows an
              Update button next to Settings.
            </span>
          </Checkbox>
          <div className="flex items-center gap-3">
            <Button onClick={() => checkForUpdates()} disabled={updates.status !== "idle"}>
              {updates.status === "checking" ? <LoaderCircle size={12} className="animate-spin" /> : <RefreshCw size={12} />}
              Check now
            </Button>
            <span className="text-xs text-faint">
              {updates.error
                ? <span className="text-removed">Couldn't check: {updates.error}</span>
                : updates.available
                  ? `worki ${updates.available.version} is available. Use Update next to Settings.`
                  : updates.checkedAt
                    ? "worki is up to date."
                    : null}
            </span>
          </div>
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

export function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-3">
      <h3 className="text-[11px] font-semibold tracking-wide text-faint uppercase">{title}</h3>
      {children}
    </div>
  );
}
