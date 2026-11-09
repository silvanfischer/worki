import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { FolderGit2, GitBranchPlus } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";

import { AddRemoteProjectDialog } from "./components/AddRemoteProjectDialog";
import { CommandPalette } from "./components/CommandPalette";
import { NewWorktreeDialog } from "./components/NewWorktreeDialog";
import { ProjectSettingsDialog } from "./components/ProjectSettingsDialog";
import { RemoveWorktreeDialog } from "./components/RemoveWorktreeDialog";
import { SaveEnvironmentDialog } from "./components/SaveEnvironmentDialog";
import { SettingsView } from "./components/SettingsView";
import { Sidebar } from "./components/Sidebar";
import { Toaster } from "./components/Toaster";
import { Button } from "./components/ui";
import { WorktreeView } from "./components/WorktreeView";
import { api, errorMessage, isolationLabel, type SessionInfo } from "./lib/api";
import { keys, useAddProject, useIsolationSettings, useProjects, useWorktrees } from "./lib/queries";
import { useUi } from "./lib/store";
import { useApplyTheme } from "./lib/theme";
import { toast } from "./lib/toast";

export function App() {
  const { data: projects = [], isFetched } = useProjects();
  const { selectedProjectId, selectedWorktreePath, select, dialog, openDialog, closeDialog } = useUi();
  const project = projects.find((p) => p.id === selectedProjectId) ?? projects[0];
  const { data: worktrees = [] } = useWorktrees(project?.id ?? "");
  const worktree = worktrees.find((w) => w.path === selectedWorktreePath) ?? worktrees[0];
  const qc = useQueryClient();
  useApplyTheme();
  const [palette, setPalette] = useState(false);

  useEffect(() => {
    const unlisten = listen<SessionInfo>("session-changed", ({ payload }) => {
      const before = qc.getQueryData<SessionInfo[]>(keys.sessions)?.find((s) => s.id === payload.id);
      const sessions =
        qc.setQueryData<SessionInfo[]>(keys.sessions, (old) =>
          old?.some((s) => s.id === payload.id)
            ? old.map((s) => (s.id === payload.id ? payload : s))
            : [...(old ?? []), payload],
        ) ?? [];
      if (payload.activity === "waiting" && before?.activity !== "waiting") notifyWaiting(payload);
      const waiting = sessions.filter((s) => s.state.kind === "running" && s.activity === "waiting").length;
      getCurrentWindow()
        .setBadgeCount(waiting || undefined)
        .catch(() => {});
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [qc]);

  useEffect(() => {
    const refresh = (n: number) => {
      if (n > 0) qc.invalidateQueries({ queryKey: keys.sessions });
    };
    api
      .restoreRemoteSessions()
      .then(refresh)
      .catch((e) => toast(`Couldn't reconnect remote sessions: ${errorMessage(e)}`));
    if (useUi.getState().reopenSessions)
      api
        .restoreAgentSessions()
        .then((report) => {
          refresh(report.started);
          for (const failure of report.failed) toast(`Couldn't reopen a session: ${failure}`);
        })
        .catch((e) => toast(`Couldn't reopen sessions: ${errorMessage(e)}`));
  }, [qc]);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.metaKey && e.key === ",") {
        e.preventDefault();
        openDialog({ kind: "settings", tab: "general" });
        return;
      }
      if (e.metaKey && !e.shiftKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPalette((open) => !open);
        return;
      }
      if (e.metaKey && !e.shiftKey && e.key.toLowerCase() === "j") {
        e.preventDefault();
        useUi.getState().toggleChanges();
        return;
      }
      if (!e.metaKey || !project) return;
      if (e.shiftKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        openDialog({ kind: "newWorktree", projectId: project.id });
      } else if (!e.shiftKey && /^[1-9]$/.test(e.key)) {
        const wt = worktrees[Number(e.key) - 1];
        if (wt) {
          e.preventDefault();
          select(project.id, wt.path);
        }
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [project, worktrees, openDialog, select]);

  const dialogProject = dialog && "projectId" in dialog ? projects.find((p) => p.id === dialog.projectId) : undefined;

  return (
    <div className="flex h-full">
      <Sidebar />
      <main className="min-w-0 flex-1 border-l border-line bg-panel">
        {project && worktree ? (
          <WorktreeView key={worktree.path} project={project} worktree={worktree} />
        ) : isFetched && projects.length === 0 ? (
          <Welcome />
        ) : project ? (
          <EmptyProject onCreate={() => openDialog({ kind: "newWorktree", projectId: project.id })} />
        ) : (
          <div data-tauri-drag-region className="h-full" />
        )}
      </main>

      {dialog?.kind === "newWorktree" && dialogProject && (
        <NewWorktreeDialog project={dialogProject} onClose={closeDialog} />
      )}
      {dialog?.kind === "settings" && <SettingsView tab={dialog.tab} />}
      {dialog?.kind === "projectSettings" && dialogProject && (
        <ProjectSettingsDialog project={dialogProject} onClose={closeDialog} />
      )}
      {dialog?.kind === "addRemoteProject" && <AddRemoteProjectDialog onClose={closeDialog} />}
      {palette && <CommandPalette onClose={() => setPalette(false)} />}
      <Toaster />
      {dialog?.kind === "saveEnvironment" && dialogProject && (
        <SaveEnvironmentDialog project={dialogProject} path={dialog.path} onClose={closeDialog} />
      )}
      {dialog?.kind === "removeWorktree" && dialogProject && (
        <RemoveWorktreeDialog project={dialogProject} path={dialog.path} onClose={closeDialog} />
      )}
    </div>
  );
}

function Welcome() {
  const addProject = useAddProject();
  const openDialog = useUi((s) => s.openDialog);
  const { data: isolation } = useIsolationSettings();
  const current = isolation?.default;
  return (
    <Centered>
      <FolderGit2 size={28} className="text-muted" />
      <h1 className="text-lg font-semibold">Welcome to worki</h1>
      <p className="max-w-sm text-center text-xs leading-relaxed text-muted">
        worki gives each agent its own git worktree, so several can work on one repository at the same time.
      </p>
      <ol className="flex w-[min(420px,90vw)] flex-col gap-2">
        <Step n={1} title="Choose where agents run">
          <span className="text-xs text-muted">
            Now: {current ? isolationLabel(current) : "…"}. Isolation keeps agents away from the rest of your Mac.
          </span>
          <Button className="self-start" onClick={() => openDialog({ kind: "settings", tab: "isolation" })}>
            Choose isolation…
          </Button>
        </Step>
        <Step n={2} title="Add a repository">
          <div className="flex gap-2">
            <Button variant="primary" onClick={() => addProject.mutate()}>
              Add repository…
            </Button>
            <Button onClick={() => openDialog({ kind: "addRemoteProject" })}>Add remote (SSH)…</Button>
          </div>
          {addProject.error && <p className="text-xs text-removed">{String(addProject.error)}</p>}
        </Step>
        <Step n={3} title="Create a worktree and start an agent">
          <span className="text-xs text-muted">
            <kbd>⌘⇧N</kbd> creates a worktree, <kbd>⌘T</kbd> starts your default agent in it, <kbd>⌘K</kbd> finds
            everything else.
          </span>
        </Step>
      </ol>
    </Centered>
  );
}

function Step({ n, title, children }: { n: number; title: string; children: ReactNode }) {
  return (
    <li className="flex gap-3 rounded-lg border border-line p-3">
      <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-accent text-[11px] font-semibold text-accent-fg">
        {n}
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <span className="text-[13px] font-medium">{title}</span>
        {children}
      </div>
    </li>
  );
}

function EmptyProject({ onCreate }: { onCreate: () => void }) {
  return (
    <Centered>
      <GitBranchPlus size={28} className="text-muted" />
      <p className="max-w-xs text-center text-xs leading-relaxed text-muted">
        Each worktree is a separate checkout on its own branch, where agents can work without getting in each
        other's way.
      </p>
      <Button variant="primary" onClick={onCreate}>
        New worktree
      </Button>
      <span className="text-[11px] text-faint">⌘⇧N</span>
    </Centered>
  );
}

function Centered({ children }: { children: ReactNode }) {
  return (
    <div data-tauri-drag-region className="flex h-full flex-col items-center justify-center gap-3">
      {children}
    </div>
  );
}

/** A macOS notification when an agent starts waiting while worki is in the background. */
async function notifyWaiting(session: SessionInfo) {
  if (document.hasFocus()) return;
  let granted = await isPermissionGranted().catch(() => false);
  if (!granted) granted = (await requestPermission().catch(() => "denied")) === "granted";
  if (!granted) return;
  const where = session.worktreePath.split("/").pop() ?? "";
  sendNotification({
    title: `${session.title} is waiting`,
    body: session.attention ?? `In ${where}`,
  });
}
