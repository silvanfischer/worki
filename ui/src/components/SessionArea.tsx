import { useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import { Bot, LoaderCircle, Plus, RotateCcw, Sparkles, SquareTerminal, X } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useState } from "react";

import { api, errorMessage, type Project, type SessionInfo, skipPermissionsFlag, type Worktree } from "../lib/api";
import { onCommand } from "../lib/commands";
import { keys, useAgentSettings, useSessions } from "../lib/queries";
import { useUi } from "../lib/store";
import { formatTokens, totalUsage, usageDetails } from "../lib/usage";
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
  const helperId = useUi((s) => s.askSession[worktree.path]);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState<string | null>(null);

  const sessions = allSessions.filter((s) => s.worktreePath === worktree.path);
  const active = sessions.find((s) => s.id === activeId) ?? sessions[sessions.length - 1];
  const profiles = agents?.profiles ?? [];
  const defaultProfile = profiles.find((p) => p.id === agents?.defaultProfile) ?? profiles[0];

  const spawn = useCallback(
    async (profileId: string | null, resumeId: string | null = null) => {
      setError(null);
      setStarting(
        resumeId ? "Resuming…" : `Starting ${profiles.find((p) => p.id === profileId)?.name ?? "shell"}…`,
      );
      try {
        const info = await api.spawnSession(
          project.id,
          worktree.path,
          profileId,
          INITIAL_COLS,
          INITIAL_ROWS,
          resumeId,
        );
        qc.setQueryData<SessionInfo[]>(keys.sessions, (old = []) =>
          old.some((s) => s.id === info.id) ? old : [...old, info],
        );
        setActive(worktree.path, info.id);
      } catch (e) {
        setError(errorMessage(e));
      } finally {
        setStarting(null);
      }
    },
    [project.id, worktree.path, qc, setActive, profiles],
  );

  async function close(session: SessionInfo) {
    await api.closeSession(session.id).catch(() => {});
    qc.setQueryData<SessionInfo[]>(keys.sessions, (old = []) => old.filter((s) => s.id !== session.id));
  }

  const resumes = (session: SessionInfo) =>
    profiles.find((p) => p.id === session.profileId)?.command.trim().split(/\s+/)[0] === "claude";

  async function restart(session: SessionInfo) {
    await close(session);
    await spawn(session.profileId, resumes(session) ? session.id : null);
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

  const sessionIds = sessions.map((s) => s.id).join(",");
  useEffect(() => {
    const ids = sessionIds ? sessionIds.split(",") : [];
    const step = (by: number) => {
      if (ids.length === 0) return;
      const at = Math.max(0, ids.indexOf(active?.id ?? ""));
      setActive(worktree.path, ids[(at + by + ids.length) % ids.length]);
    };
    const offCommand = onCommand((c) => {
      if (c.kind === "newSession") spawn(c.profileId);
      else if (c.kind === "nextTab") step(c.step);
    });
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey) return;
      if (e.key.toLowerCase() === "t") {
        e.preventDefault();
        spawn(e.shiftKey ? null : (defaultProfile?.id ?? null));
      } else if (e.shiftKey && (e.key === "[" || e.key === "{")) {
        e.preventDefault();
        step(-1);
      } else if (e.shiftKey && (e.key === "]" || e.key === "}")) {
        e.preventDefault();
        step(1);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      offCommand();
      window.removeEventListener("keydown", onKey);
    };
  }, [spawn, sessionIds, active?.id, defaultProfile?.id, setActive, worktree.path]);

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
        <p className="text-[11px] text-faint">
          {starting ?? (
            <>
              <Kbd>⌘T</Kbd> starts {defaultProfile?.name ?? "an agent"}, <Kbd>⌘⇧T</Kbd> a shell
            </>
          )}
        </p>
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
              yolo={isYolo(profiles.find((p) => p.id === s.profileId)?.command)}
              helper={s.id === helperId}
              selected={s.id === active?.id}
              onSelect={() => setActive(worktree.path, s.id)}
              onClose={() => close(s)}
            />
          ))}
          {starting && (
            <span className="flex h-7 shrink-0 items-center gap-2 rounded-md px-2.5 text-muted">
              <LoaderCircle size={12} className="animate-spin" />
              {starting}
            </span>
          )}
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
              <Button
                variant="primary"
                onClick={() => restart(active)}
                title={resumes(active) ? "Starts Claude again in this conversation" : undefined}
              >
                <RotateCcw size={12} />
                {resumes(active) ? "Continue conversation" : "Start again"}
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
    <div className="absolute inset-x-3 bottom-3 z-20 flex items-center gap-3 rounded-lg border border-line bg-elevated px-3 py-2 shadow-lg">
      <span className="flex-1 text-xs text-muted">{message}</span>
      {children}
    </div>
  );
}

function isYolo(command: string | undefined): boolean {
  const flag = command && skipPermissionsFlag(command);
  return !!flag && command.split(/\s+/).includes(flag);
}

function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="rounded border border-line px-1 font-sans text-[10px] text-muted">{children}</kbd>;
}

function SessionTab({
  session,
  yolo,
  helper,
  selected,
  onSelect,
  onClose,
}: {
  session: SessionInfo;
  yolo: boolean;
  helper: boolean;
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
        helper && "border border-dashed border-accent/50 text-[12px]",
        selected ? "bg-selected text-fg" : "text-muted hover:bg-hover hover:text-fg",
      )}
      title={helper ? `Helper: runs the “Ask agent” tasks with ${session.title}` : undefined}
    >
      <ActivityDot
        state={sessionState(session)}
        title={session.attention ? `Waiting: ${session.attention}` : stateLabel[sessionState(session)]}
      />
      {helper && <Sparkles size={12} className="-mx-0.5 text-accent" />}
      <span className={clsx("max-w-40 truncate", helper && "italic")}>{helper ? "Helper" : session.title}</span>
      {yolo && (
        <span
          className="rounded bg-warn/15 px-1 text-[10px] font-semibold text-warn"
          title="YOLO mode: runs commands and edits without asking"
        >
          YOLO
        </span>
      )}
      <UsageBadge session={session} />
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

function UsageBadge({ session }: { session: SessionInfo }) {
  const total = totalUsage(session.usage);
  if (!total) return null;
  return (
    <span className="font-mono text-[10px] text-faint" title={`Tokens used\n${usageDetails(total)}`}>
      {formatTokens(total.tokens)}
    </span>
  );
}
