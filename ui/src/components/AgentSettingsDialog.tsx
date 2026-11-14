import { useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import { Bot, MessageSquare, Pencil, Plus, SlidersHorizontal, SquareTerminal, Star, Trash2 } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";

import {
  type AgentProfile,
  type AgentSettings,
  api,
  errorMessage,
  skipPermissionsFlag,
  withSkipPermissions,
} from "../lib/api";
import { keys } from "../lib/queries";
import { useUi } from "../lib/store";
import { ClaudeSettingsDialog } from "./ClaudeSettingsDialog";
import { Group } from "./GeneralSettings";
import { Button, Checkbox, Dialog, ErrorText, Field, IconButton, Input, NestedDialog } from "./ui";

const BUILT_IN = ["claude", "codex"];

/** Claude Code and Codex are always there; other agents are added below them. Saved as you change it. */
export function AgentSettingsDialog({ settings, onClose }: { settings: AgentSettings; onClose: () => void }) {
  const qc = useQueryClient();
  const { profiles, setProfiles, defaultId, setDefaultId, error, setError } = useSavedProfiles(settings);
  const [editingClaude, setEditingClaude] = useState(false);
  const [editing, setEditing] = useState<AgentProfile | null>(null);

  const claude = profiles.find((p) => p.id === "claude");
  const codex = profiles.find((p) => p.id === "codex");
  const others = profiles.filter((p) => !BUILT_IN.includes(p.id));

  const update = (agent: AgentProfile) =>
    setProfiles((ps) =>
      ps.some((p) => p.id === agent.id) ? ps.map((p) => (p.id === agent.id ? agent : p)) : [...ps, agent],
    );
  const setCommand = (agent: AgentProfile) => (command: string) => update({ ...agent, command });
  const defaultMark = (agent: AgentProfile) => (
    <DefaultMark isDefault={agent.id === defaultId} onMakeDefault={() => setDefaultId(agent.id)} />
  );

  async function setRemoteControl(on: boolean) {
    try {
      await api.setRemoteControl(on);
      setError(null);
      await qc.invalidateQueries({ queryKey: keys.agentSettings });
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <Dialog title="Agents" onClose={onClose} width={600}>
      <Group title="Starting agents">
        <OpenAs />
      </Group>

      <Group title="Claude Code and Codex">
        {claude && (
          <AgentBox title="Claude Code" isDefault={claude.id === defaultId} aside={defaultMark(claude)}>
            <CommandField agent={claude} placeholder="claude --model opus" onChange={setCommand(claude)} />
            <Yolo agent={claude} onChange={setCommand(claude)} />
            <Checkbox checked={settings.remoteControl} onChange={setRemoteControl}>
              <span className="text-[13px]">Remote Control</span>
              <span className="block text-xs leading-relaxed text-faint">
                Continue sessions from claude.ai/code or the Claude app. Docker Sandboxes also need{" "}
                <span className="font-mono">sbx settings set claude.remoteControl true</span>.
              </span>
            </Checkbox>
            <div className="flex flex-col gap-1.5">
              <Button className="self-start" onClick={() => setEditingClaude(true)}>
                <SlidersHorizontal size={12} />
                Settings for isolated sessions…
              </Button>
              <span className="text-xs leading-relaxed text-faint">
                For User Isolation, Docker Sandboxes, Docker and Podman.
              </span>
            </div>
          </AgentBox>
        )}
        {codex && (
          <AgentBox title="Codex" isDefault={codex.id === defaultId} aside={defaultMark(codex)}>
            <CommandField agent={codex} placeholder="codex --model o3" onChange={setCommand(codex)} />
            <Yolo agent={codex} onChange={setCommand(codex)} />
          </AgentBox>
        )}
      </Group>

      <Group title="Other agents">
        {others.length > 0 ? (
          <div className="flex flex-col divide-y divide-line rounded-lg border border-line">
            {others.map((p) => (
              <div key={p.id} className="flex items-center gap-3 px-3 py-2">
                <Bot size={14} className="shrink-0 text-faint" />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13px]">{p.name}</div>
                  <div className="truncate font-mono text-[11px] text-faint">{p.command}</div>
                </div>
                {defaultMark(p)}
                <IconButton label="Edit agent" onClick={() => setEditing(p)}>
                  <Pencil size={13} />
                </IconButton>
                <IconButton label="Remove agent" onClick={() => setProfiles((ps) => ps.filter((x) => x.id !== p.id))}>
                  <Trash2 size={13} />
                </IconButton>
              </div>
            ))}
          </div>
        ) : (
          <p className="-mt-1 text-xs leading-relaxed text-faint">
            Any command-line agent, such as Aider or Gemini CLI.
          </p>
        )}
        <Button
          className="self-start"
          onClick={() => setEditing({ id: crypto.randomUUID(), name: "", command: "", install: null })}
        >
          <Plus size={13} />
          Add agent
        </Button>
      </Group>

      <ErrorText>{error}</ErrorText>
      {editingClaude && (
        <ClaudeSettingsDialog settings={settings.claudeSettings} onClose={() => setEditingClaude(false)} />
      )}
      {editing && (
        <AgentDialog
          agent={editing}
          isNew={!profiles.some((p) => p.id === editing.id)}
          onSave={(agent) => {
            update(agent);
            setEditing(null);
          }}
          onClose={() => setEditing(null)}
        />
      )}
    </Dialog>
  );
}

/** The agents as edited here, saved shortly after each change. Incomplete ones aren't saved. */
function useSavedProfiles(settings: AgentSettings) {
  const qc = useQueryClient();
  const [profiles, setProfiles] = useState(settings.profiles);
  const [chosenDefault, setDefaultId] = useState(settings.defaultProfile);
  const [error, setError] = useState<string | null>(null);
  const defaultId = profiles.some((p) => p.id === chosenDefault) ? chosenDefault : (profiles[0]?.id ?? null);

  const saved = useRef(JSON.stringify({ profiles: settings.profiles, defaultProfile: settings.defaultProfile }));
  useEffect(() => {
    const complete = profiles
      .map((p) => ({ ...p, name: p.name.trim(), command: p.command.trim() }))
      .filter((p) => p.name && p.command);
    const snapshot = JSON.stringify({ profiles: complete, defaultProfile: defaultId });
    if (snapshot === saved.current) return;
    const timer = setTimeout(async () => {
      try {
        await api.saveAgentProfiles(complete, defaultId);
        saved.current = snapshot;
        setError(null);
        await qc.invalidateQueries({ queryKey: keys.agentSettings });
      } catch (e) {
        setError(errorMessage(e));
      }
    }, 400);
    return () => clearTimeout(timer);
  }, [profiles, defaultId, qc]);

  return { profiles, setProfiles, defaultId, setDefaultId, error, setError };
}

function OpenAs() {
  const agentView = useUi((s) => s.agentView);
  const setAgentView = useUi((s) => s.setAgentView);
  const options = [
    { id: "chat", label: "Chat", icon: <MessageSquare size={13} /> },
    { id: "terminal", label: "Terminal", icon: <SquareTerminal size={13} /> },
  ] as const;
  return (
    <Field
      label="Claude Code and Codex open as"
      hint="The + menu offers both."
    >
      <div className="flex w-fit rounded-md border border-line p-0.5">
        {options.map(({ id, label, icon }) => (
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
  );
}

function AgentBox({
  title,
  isDefault,
  aside,
  children,
}: {
  title: string;
  isDefault: boolean;
  aside: ReactNode;
  children: ReactNode;
}) {
  return (
    <div
      className={clsx(
        "flex flex-col gap-3 rounded-lg border bg-panel p-3",
        isDefault ? "border-accent/50" : "border-line",
      )}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="text-[13px] font-semibold">{title}</span>
        {aside}
      </div>
      {children}
    </div>
  );
}

function DefaultMark({ isDefault, onMakeDefault }: { isDefault: boolean; onMakeDefault: () => void }) {
  if (isDefault) {
    return (
      <span className="flex h-6 shrink-0 items-center gap-1 rounded-full bg-accent/15 px-2 text-[11px] font-medium text-accent">
        <Star size={11} className="fill-current" />
        Default · ⌘T
      </span>
    );
  }
  return (
    <Button variant="ghost" onClick={onMakeDefault} className="h-6 shrink-0 px-2 text-[11px]">
      <Star size={11} />
      Make default
    </Button>
  );
}

function CommandField({
  agent,
  placeholder,
  onChange,
}: {
  agent: AgentProfile;
  placeholder: string;
  onChange: (command: string) => void;
}) {
  return (
    <Field label="Command">
      <Input
        value={agent.command}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="font-mono text-xs"
      />
    </Field>
  );
}

function Yolo({ agent, onChange }: { agent: AgentProfile; onChange: (command: string) => void }) {
  const flag = skipPermissionsFlag(agent.command);
  if (!flag) return null;
  return (
    <Checkbox
      checked={agent.command.split(/\s+/).includes(flag)}
      onChange={(on) => onChange(withSkipPermissions(agent.command, on))}
    >
      <span className="text-[13px]">YOLO mode</span>
      <span className="block text-xs text-faint">
        Adds <span className="font-mono">{flag}</span>: no permission prompts. Best with isolation.
      </span>
    </Checkbox>
  );
}

function AgentDialog({
  agent,
  isNew,
  onSave,
  onClose,
}: {
  agent: AgentProfile;
  isNew: boolean;
  onSave: (agent: AgentProfile) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState(agent.name);
  const [command, setCommand] = useState(agent.command);
  const [install, setInstall] = useState(agent.install ?? "");
  const complete = name.trim() !== "" && command.trim() !== "";

  function save() {
    if (complete) onSave({ ...agent, name: name.trim(), command: command.trim(), install: install.trim() || null });
  }

  return (
    <NestedDialog
      title={isNew ? "Add agent" : `Edit ${agent.name}`}
      onClose={onClose}
      width={480}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={save} disabled={!complete}>
            {isNew ? "Add" : "Save"}
          </Button>
        </>
      }
    >
      <form
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <Field label="Name">
          <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Aider" />
        </Field>
        <Field label="Command" hint="Runs in the worktree through your login shell.">
          <Input
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            placeholder="aider --model sonnet"
            className="font-mono text-xs"
          />
        </Field>
        <Field
          label="Install command"
          hint="Optional. Installs the agent in isolated environments."
        >
          <Input
            value={install}
            onChange={(e) => setInstall(e.target.value)}
            placeholder="pip install aider-chat"
            className="font-mono text-xs"
          />
        </Field>
        <button type="submit" hidden />
      </form>
    </NestedDialog>
  );
}
