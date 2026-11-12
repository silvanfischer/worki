import { useQueryClient } from "@tanstack/react-query";
import { RotateCcw, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { api, errorMessage, type Project, type SessionInfo, type Worktree } from "../lib/api";
import { keys, useWorktreeShell } from "../lib/queries";
import { useUi } from "../lib/store";
import { ResizeHandle } from "./ResizeHandle";
import { TerminalView } from "./TerminalView";
import { Button, ErrorText, IconButton } from "./ui";

/** Your own shell in the worktree, below its sessions; it runs outside the worktree's isolation. */
export function TerminalPanel({ project, worktree }: { project: Project; worktree: Worktree }) {
  const qc = useQueryClient();
  const { data: shell, isFetched } = useWorktreeShell(worktree.path);
  const height = useUi((s) => s.panelWidths.terminal);
  const toggleTerminal = useUi((s) => s.toggleTerminal);
  const [error, setError] = useState<string | null>(null);
  const starting = useRef(false);

  const start = useCallback(async () => {
    if (starting.current) return;
    starting.current = true;
    setError(null);
    try {
      const info = await api.spawnWorktreeShell(project.id, worktree.path, 120, 12);
      qc.setQueryData<SessionInfo[]>(keys.sessions, (old = []) =>
        old.some((s) => s.id === info.id) ? old : [...old, info],
      );
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      starting.current = false;
    }
  }, [project.id, worktree.path, qc]);

  async function startAgain(old: SessionInfo) {
    await api.closeSession(old.id).catch(() => {});
    qc.setQueryData<SessionInfo[]>(keys.sessions, (all = []) => all.filter((s) => s.id !== old.id));
    await start();
  }

  useEffect(() => {
    if (isFetched && !shell && !error) start();
  }, [isFetched, shell, error, start]);

  const where = project.remote ? `on ${shell?.runAs ?? "the remote host"}` : "on this Mac, not isolated";

  return (
    <div className="relative flex max-h-[80%] shrink-0 flex-col border-t border-line" style={{ height }}>
      <ResizeHandle panel="terminal" edge="top" />
      <div className="flex h-7 shrink-0 items-center gap-2 border-b border-line bg-bar pr-1 pl-3">
        <span className="text-xs font-medium">Terminal</span>
        <span className="min-w-0 flex-1 truncate text-[11px] text-faint">{where}</span>
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
