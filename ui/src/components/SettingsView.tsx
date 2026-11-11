import clsx from "clsx";
import { AppWindow, Bot, Camera, GitBranch, KeyRound, Palette, Server, Settings, Shield, X } from "lucide-react";
import { type ReactNode, useEffect } from "react";

import { useAgentSettings, useIsolationSettings } from "../lib/queries";
import { type SettingsTab, useUi } from "../lib/store";
import { AgentSettingsDialog } from "./AgentSettingsDialog";
import { AppearanceSettings } from "./AppearanceSettings";
import { GeneralSettings } from "./GeneralSettings";
import { GitSettings } from "./GitSettings";
import { IsolationDialog } from "./IsolationDialog";
import { OpenInSettings } from "./OpenInSettings";
import { SecretsEditor } from "./SecretsEditor";
import { SnapshotSettings } from "./SnapshotSettings";
import { SshHostsSettings } from "./SshHostsSettings";
import { EmbeddedDialog, IconButton } from "./ui";

const tabs: { id: SettingsTab; label: string; icon: ReactNode }[] = [
  { id: "general", label: "General", icon: <Settings size={14} /> },
  { id: "appearance", label: "Appearance", icon: <Palette size={14} /> },
  { id: "agents", label: "Agents", icon: <Bot size={14} /> },
  { id: "git", label: "Git", icon: <GitBranch size={14} /> },
  { id: "secrets", label: "Secrets", icon: <KeyRound size={14} /> },
  { id: "isolation", label: "Isolation", icon: <Shield size={14} /> },
  { id: "snapshots", label: "Snapshots", icon: <Camera size={14} /> },
  { id: "sshHosts", label: "SSH hosts", icon: <Server size={14} /> },
  { id: "openIn", label: "Open in", icon: <AppWindow size={14} /> },
];

/** All settings in one window. Changes are saved as they're made. */
export function SettingsView({ tab }: { tab: SettingsTab }) {
  const openDialog = useUi((s) => s.openDialog);
  const closeDialog = useUi((s) => s.closeDialog);
  const { data: agents } = useAgentSettings();
  const { data: isolation } = useIsolationSettings();
  const select = (next: SettingsTab) => openDialog({ kind: "settings", tab: next });

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && closeDialog();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [closeDialog]);

  return (
    <div
      data-overlay
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/25 p-8 backdrop-blur-[1px]"
      onMouseDown={(e) => e.target === e.currentTarget && closeDialog()}
    >
      <div
        role="dialog"
        aria-label="Settings"
        className="flex h-[min(760px,88vh)] w-[min(980px,94vw)] overflow-hidden rounded-xl border border-line bg-elevated shadow-2xl"
      >
        <nav className="flex w-52 shrink-0 flex-col gap-0.5 border-r border-line bg-panel/60 p-2">
          <div className="px-2 pt-2 pb-3 text-[11px] font-semibold tracking-wide text-faint uppercase">Settings</div>
          {tabs.map((t) => (
            <button
              key={t.id}
              onClick={() => select(t.id)}
              className={clsx(
                "flex h-8 items-center gap-2 rounded-md px-2 text-left text-[13px]",
                t.id === tab ? "bg-selected font-medium text-fg" : "text-muted hover:bg-hover hover:text-fg",
              )}
            >
              {t.icon}
              {t.label}
            </button>
          ))}
        </nav>

        <div className="relative flex min-w-0 flex-1 flex-col">
          <IconButton label="Close settings" onClick={closeDialog} className="absolute top-4 right-4 z-10">
            <X size={15} />
          </IconButton>
          <EmbeddedDialog.Provider value>
            {tab === "general" && <GeneralSettings />}
            {tab === "appearance" && <AppearanceSettings />}
            {tab === "git" && <GitSettings />}
            {tab === "agents" && agents && <AgentSettingsDialog settings={agents} onClose={closeDialog} />}
            {tab === "secrets" && (
              <section className="flex min-h-0 flex-1 flex-col">
                <h2 className="px-6 pt-5 pb-3 text-[15px] font-semibold">Secrets</h2>
                <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-6">
                  <SecretsEditor />
                </div>
              </section>
            )}
            {tab === "isolation" && isolation && (
              <IsolationDialog current={isolation.default} onClose={closeDialog} />
            )}
            {tab === "snapshots" && <SnapshotSettings />}
            {tab === "sshHosts" && <SshHostsSettings />}
            {tab === "openIn" && <OpenInSettings />}
          </EmbeddedDialog.Provider>
        </div>
      </div>
    </div>
  );
}
