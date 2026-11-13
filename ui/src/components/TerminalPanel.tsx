import { useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import { Plus, RotateCcw, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { api, errorMessage, type Project, type SessionInfo, type Worktree } from "../lib/api";
import { keys, useWorktreeShells } from "../lib/queries";
import { useUi } from "../lib/store";
import { ResizeHandle } from "./ResizeHandle";
import { TerminalView } from "./TerminalView";
import { Button, ErrorText, IconButton } from "./ui";

/** Your own shells in the worktree, as tabs below its sessions; they run outside the worktree's
 * isolation. */
export function TerminalPanel({ project, worktree }: { project: Project; worktree: Worktree }) {
  const qc = useQueryClient();
  const { data: shells = [], isFetched } = useWorktreeShells(worktree.path);
  const height = useUi((s) => s.panelWidths.terminal);
  const toggleTerminal = useUi((s) => s.toggleTerminal);
  const activeId = useUi((s) => s.activeTerminal[worktree.path]);
  const setActive = useUi((s) => s.setActiveTerminal);
  const [error, setError] = useState<string | null>(null);
  const starting = useRef(false);
  const shell = shells.find((s) => s.id === activeId) ?? shells.at(-1);

  const start = useCallback(async () => {
    if (starting.current) return;
    starting.current = true;
    setError(null);
    try {
      const info = await api.spawnWorktreeShell(project.id, worktree.path, 120, 12);
      qc.setQueryData<SessionInfo[]>(keys.sessions, (old = []) =>
        old.some((s) => s.id === info.id) ? old : [...old, info],
      );
      setActive(worktree.path, info.id);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      starting.current = false;
    }
  }, [project.id, worktree.path, qc, setActive]);

  async function close(old: SessionInfo) {
    if (shells.length === 1) toggleTerminal();
    await api.closeSession(old.id).catch(() => {});
    qc.setQueryData<SessionInfo[]>(keys.sessions, (all = []) => all.filter((s) => s.id !== old.id));
  }

  async function startAgain(old: SessionInfo) {
    await api.closeSession(old.id).catch(() => {});
    qc.setQueryData<SessionInfo[]>(keys.sessions, (all = []) => all.filter((s) => s.id !== old.id));
    await start();
  }

  useEffect(() => {
    if (isFetched && shells.length === 0 && !error) start();
  }, [isFetched, shells.length, error, start]);

  const where = project.remote ? `on ${shell?.runAs ?? "the remote host"}` : "on this Mac, not isolated";

  return (
    <div className="relative flex max-h-[80%] shrink-0 flex-col border-t border-line" style={{ height }}>
      <ResizeHandle panel="terminal" edge="top" />
      <div className="flex h-7 shrink-0 items-center gap-0.5 border-b border-line bg-bar pr-1 pl-1.5">
        {shells.map((s, i) => (
          <ShellTab
            key={s.id}
            label={i === 0 ? "Terminal" : `Terminal ${i + 1}`}
            selected={s.id === shell?.id}
            onSelect={() => setActive(worktree.path, s.id)}
            onClose={() => close(s)}
          />
        ))}
        <IconButton label="New terminal" onClick={start}>
          <Plus size={13} />
        </IconButton>
        <span className="min-w-0 flex-1 truncate px-2 text-[11px] text-faint">{where}</span>
        <IconButton label="Hide terminal (⌃`)" onClick={toggleTerminal}>
          <X size={13} />
        </IconButton>
      </div>
      <div className="relative min-h-0 flex-1">
        {shell && <TerminalView key={`${shell.id}-${shell.createdAt}`} sessionId={shell.id} active={false} />}
        {(error || shell?.state.kind === "exited") && (
          <div className="absolute inset-0 z-10 flex items-center justify-center gap-3 bg-panel/80 p-3">
            {error ? <ErrorText>{error}</ErrorText> : <span className="text-xs text-muted">The shell exited.</span>}
            <Button onClick={() => (shell ? startAgain(shell) : start())}>
              <RotateCcw size={12} />
              Start again
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

function ShellTab({
  label,
  selected,
  onSelect,
  onClose,
}: {
  label: string;
  selected: boolean;
  onSelect: () => void;
  onClose: () => void;
}) {
  return (
    <div
      className={clsx(
        "group flex h-5.5 shrink-0 items-center gap-0.5 rounded-md pr-0.5 pl-2 text-xs",
        selected ? "bg-selected font-medium text-fg" : "text-muted hover:bg-hover hover:text-fg",
      )}
    >
      <button onClick={onSelect}>{label}</button>
      <button
        aria-label={`Close ${label}`}
        onClick={onClose}
        className={clsx(
          "flex size-4 items-center justify-center rounded hover:bg-hover",
          selected ? "opacity-100" : "opacity-0 group-hover:opacity-100",
        )}
      >
        <X size={10} />
      </button>
    </div>
  );
}
