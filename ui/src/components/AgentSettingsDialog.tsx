import { useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { type AgentProfile, type AgentSettings, api, errorMessage } from "../lib/api";
import { keys } from "../lib/queries";
import { Button, Dialog, ErrorText, IconButton, Input } from "./ui";

/** Saved as you type; rows missing a name or command are kept here until they're complete. */
export function AgentSettingsDialog({ settings, onClose }: { settings: AgentSettings; onClose: () => void }) {
  const qc = useQueryClient();
  const [profiles, setProfiles] = useState<AgentProfile[]>(settings.profiles);
  const [defaultProfile, setDefaultProfile] = useState(settings.defaultProfile);
  const [error, setError] = useState<string | null>(null);

  const update = (i: number, patch: Partial<AgentProfile>) =>
    setProfiles((ps) => ps.map((p, j) => (j === i ? { ...p, ...patch } : p)));

  const saved = useRef(JSON.stringify({ profiles: settings.profiles, defaultProfile: settings.defaultProfile }));
  const incomplete = profiles.some((p) => !p.name.trim() !== !p.command.trim());

  useEffect(() => {
    const cleaned = profiles
      .map((p) => ({ ...p, name: p.name.trim(), command: p.command.trim(), install: p.install?.trim() || null }))
      .filter((p) => p.name && p.command);
    const nextDefault = cleaned.some((p) => p.id === defaultProfile) ? defaultProfile : (cleaned[0]?.id ?? null);
    const snapshot = JSON.stringify({ profiles: cleaned, defaultProfile: nextDefault });
    if (snapshot === saved.current) return;
    const timer = setTimeout(async () => {
      try {
        const { secrets } = await api.agentSettings();
        await api.saveAgentSettings({ profiles: cleaned, defaultProfile: nextDefault, secrets });
        saved.current = snapshot;
        setError(null);
        await qc.invalidateQueries({ queryKey: keys.agentSettings });
      } catch (e) {
        setError(errorMessage(e));
      }
    }, 400);
    return () => clearTimeout(timer);
  }, [profiles, defaultProfile, qc]);

  return (
    <Dialog title="Agents" onClose={onClose} width={600}>
      <p className="text-xs leading-relaxed text-muted">
        Each agent is a command run through your login shell in the worktree, so it sees the same PATH as your
        terminal. The default agent is offered first when starting a session.
      </p>
      <div className="flex flex-col gap-2">
        <div className="grid grid-cols-[52px_160px_1fr_24px] gap-2 px-1 text-[11px] font-medium text-faint">
          <span>Default</span>
          <span>Name</span>
          <span>Command</span>
        </div>
        {profiles.map((p, i) => (
          <div key={p.id} className="grid grid-cols-[52px_160px_1fr_24px] items-center gap-2 px-1">
            <input
              type="radio"
              name="default-agent"
              className="justify-self-center accent-[var(--accent)]"
              checked={defaultProfile === p.id}
              onChange={() => setDefaultProfile(p.id)}
            />
            <Input value={p.name} onChange={(e) => update(i, { name: e.target.value })} placeholder="Claude Code" />
            <Input
              value={p.command}
              onChange={(e) => update(i, { command: e.target.value })}
              placeholder="claude --model opus"
              className="font-mono text-xs"
            />
            <IconButton label="Remove agent" onClick={() => setProfiles((ps) => ps.filter((_, j) => j !== i))}>
              <Trash2 size={13} />
            </IconButton>
            <span />
            <span className="text-right text-[11px] text-faint">Install</span>
            <Input
              value={p.install ?? ""}
              onChange={(e) => update(i, { install: e.target.value })}
              placeholder="Optional: installs it for an isolated user"
              className="h-7 font-mono text-[11px]"
            />
            <span />
          </div>
        ))}
      </div>
      <Button
        variant="ghost"
        className="self-start"
        onClick={() => setProfiles((ps) => [...ps, { id: crypto.randomUUID(), name: "", command: "", install: null }])}
      >
        <Plus size={13} />
        Add agent
      </Button>

      {incomplete && <p className="text-xs text-faint">Agents need a name and a command before they're saved.</p>}
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}
