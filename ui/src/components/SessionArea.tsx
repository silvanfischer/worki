import { useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import { Bot, Plus, RotateCcw, SquareTerminal, X } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useState } from "react";

import { api, errorMessage, type Project, type SessionInfo, type Worktree } from "../lib/api";
import { keys, useAgentSettings, useSessions } from "../lib/queries";
import { useUi } from "../lib/store";
import { ActivityDot, sessionState, stateLabel } from "./ActivityDot";
import { TerminalView } from "./TerminalView";
import { Button, ErrorText, IconButton, Menu } from "./ui";

const INITIAL_COLS = 120;
const INITIAL_ROWS = 32;

export function SessionArea({ project, worktree }: { project: Project; worktree: Worktree }) {
  const qc = useQueryClient();
  const { data: allSessions = [] } = useSessions();
  const { data: agents } = useAgentSettings();
  const activeId = useUi((s) => s.activeSession[worktree.path]);
  const setActive = useUi((s) => s.setActiveSession);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);

  const sessions = allSessions.filter((s) => s.worktreePath === worktree.path);
  const active = sessions.find((s) => s.id === activeId) ?? sessions[sessions.length - 1];
  const profiles = agents?.profiles ?? [];
  const defaultProfile = profiles.find((p) => p.id === agents?.defaultProfile) ?? profiles[0];

  const spawn = useCallback(
    async (profileId: string | null) => {
      setError(null);
      setStarting(true);
      try {
        const info = await api.spawnSession(project.id, worktree.path, profileId, INITIAL_COLS, INITIAL_ROWS);
        qc.setQueryData<SessionInfo[]>(keys.sessions, (old = []) =>
          old.some((s) => s.id === info.id) ? old : [...old, info],
        );
        setActive(worktree.path, info.id);
      } catch (e) {
        setError(errorMessage(e));
      } finally {
        setStarting(false);
      }
    },
    [project.id, worktree.path, qc, setActive],
  );

  async function close(session: SessionInfo) {
    await api.closeSession(session.id).catch(() => {});
    qc.setQueryData<SessionInfo[]>(keys.sessions, (old = []) => old.filter((s) => s.id !== session.id));
  }

  async function restart(session: SessionInfo) {
    await close(session);
    await spawn(session.profileId);
  }

  async function reconnect(session: SessionInfo) {
    setError(null);
    try {
      const info = await api.reconnectSession(session.id, INITIAL_COLS, INITIAL_ROWS);
      qc.setQueryData<SessionInfo[]>(keys.sessions, (old = []) => old.map((s) => (s.id === info.id ? info : s)));
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && !e.shiftKey && e.key === "t") {
        e.preventDefault();
        spawn(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [spawn]);

  const newMenu = [
    ...profiles.map((p) => ({ label: p.name, icon: <Bot size={13} />, onSelect: () => spawn(p.id) })),
    { label: "Shell", icon: <SquareTerminal size={13} />, onSelect: () => spawn(null) },
  ];

  if (sessions.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-4 p-8">
        <div className="flex size-12 items-center justify-center rounded-xl bg-hover">
          <Bot size={22} className="text-muted" />
        </div>
        <div className="flex flex-wrap justify-center gap-2">
          {defaultProfile && (
            <Button variant="primary" onClick={() => spawn(defaultProfile.id)}>
              Start {defaultProfile.name}
            </Button>
          )}
          {profiles
            .filter((p) => p.id !== defaultProfile?.id)
            .map((p) => (
              <Button key={p.id} onClick={() => spawn(p.id)}>
                {p.name}
              </Button>
            ))}
          <Button onClick={() => spawn(null)}>
            <SquareTerminal size={13} />
            Shell
          </Button>
        </div>
        {starting && <p className="text-xs text-faint">Starting…</p>}
        <ErrorText>{error}</ErrorText>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-9 shrink-0 items-center gap-1 border-b border-line px-2">
        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
          {sessions.map((s) => (
            <SessionTab
              key={s.id}
              session={s}
              selected={s.id === active?.id}
              onSelect={() => setActive(worktree.path, s.id)}
              onClose={() => close(s)}
            />
          ))}
        </div>
        <Menu
          trigger={(toggle) => (
            <IconButton label="New session" onClick={toggle}>
              <Plus size={14} />
            </IconButton>
          )}
          items={newMenu}
        />
      </div>
      {error && (
        <div className="px-3 pt-2">
          <ErrorText>{error}</ErrorText>
        </div>
      )}
      <div className="relative min-h-0 flex-1">
        {active && <TerminalView key={`${active.id}-${active.createdAt}`} sessionId={active.id} active />}
        {active?.state.kind === "exited" &&
          (active.resumable && active.state.code !== 0 ? (
            <ExitBanner message={<>Connection to {active.runAs ?? "the host"} lost. The session keeps running there.</>}>
              <Button variant="primary" onClick={() => reconnect(active)}>
                <RotateCcw size={12} />
                Reconnect
              </Button>
              <Button variant="ghost" onClick={() => close(active)}>
                End session
              </Button>
            </ExitBanner>
          ) : (
            <ExitBanner
              message={
                <>
                  Process exited with code <span className="font-mono text-fg">{active.state.code}</span>
                </>
              }
            >
              <Button onClick={() => restart(active)}>
                <RotateCcw size={12} />
                Restart
              </Button>
              <Button variant="ghost" onClick={() => close(active)}>
                Close
              </Button>
            </ExitBanner>
          ))}
      </div>
    </div>
  );
}

function ExitBanner({ message, children }: { message: ReactNode; children: ReactNode }) {
  return (
    <div className="absolute inset-x-3 bottom-3 flex items-center gap-3 rounded-lg border border-line bg-elevated px-3 py-2 shadow-lg">
      <span className="flex-1 text-xs text-muted">{message}</span>
      {children}
    </div>
  );
}

function SessionTab({
  session,
  selected,
  onSelect,
  onClose,
}: {
  session: SessionInfo;
  selected: boolean;
  onSelect: () => void;
  onClose: () => void;
}) {
  const running = session.state.kind === "running";
  return (
    <div
      onClick={onSelect}
      onAuxClick={(e) => e.button === 1 && onClose()}
      className={clsx(
        "group flex h-7 shrink-0 items-center gap-2 rounded-md pr-1 pl-2.5",
        selected ? "bg-selected text-fg" : "text-muted hover:bg-hover hover:text-fg",
      )}
    >
      <ActivityDot
        state={sessionState(session)}
        title={session.attention ? `Waiting: ${session.attention}` : stateLabel[sessionState(session)]}
      />
      <span className="max-w-40 truncate">{session.title}</span>
      {session.runAs && (
        <span className="rounded bg-hover px-1 font-mono text-[10px] text-faint" title={`Runs as ${session.runAs}`}>
          {session.runAs}
        </span>
      )}
      <button
        aria-label="Close session"
        title={running ? "Stop and close" : "Close"}
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
        className="flex size-4 items-center justify-center rounded opacity-0 group-hover:opacity-100 hover:bg-hover"
      >
        <X size={11} />
      </button>
    </div>
  );
}
