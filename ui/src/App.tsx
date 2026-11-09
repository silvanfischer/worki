import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { FolderGit2, GitBranchPlus } from "lucide-react";
import { type ReactNode, useEffect } from "react";

import { AddRemoteProjectDialog } from "./components/AddRemoteProjectDialog";
import { NewWorktreeDialog } from "./components/NewWorktreeDialog";
import { ProjectSettingsDialog } from "./components/ProjectSettingsDialog";
import { RemoveWorktreeDialog } from "./components/RemoveWorktreeDialog";
import { SettingsView } from "./components/SettingsView";
import { Sidebar } from "./components/Sidebar";
import { Button } from "./components/ui";
import { WorktreeView } from "./components/WorktreeView";
import { api, type SessionInfo } from "./lib/api";
import { keys, useAddProject, useProjects, useWorktrees } from "./lib/queries";
import { useUi } from "./lib/store";

export function App() {
  const { data: projects = [], isFetched } = useProjects();
  const { selectedProjectId, selectedWorktreePath, select, dialog, openDialog, closeDialog } = useUi();
  const project = projects.find((p) => p.id === selectedProjectId) ?? projects[0];
  const { data: worktrees = [] } = useWorktrees(project?.id ?? "");
  const worktree = worktrees.find((w) => w.path === selectedWorktreePath) ?? worktrees[0];
  const qc = useQueryClient();

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
    api
      .restoreRemoteSessions()
      .then((n) => {
        if (n > 0) qc.invalidateQueries({ queryKey: keys.sessions });
      })
      .catch(() => {});
  }, [qc]);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.metaKey && e.key === ",") {
        e.preventDefault();
        openDialog({ kind: "settings", tab: "agents" });
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
      {dialog?.kind === "removeWorktree" && dialogProject && (
        <RemoveWorktreeDialog project={dialogProject} path={dialog.path} onClose={closeDialog} />
      )}
    </div>
  );
}

function Welcome() {
  const addProject = useAddProject();
  const openDialog = useUi((s) => s.openDialog);
  return (
    <Centered>
      <FolderGit2 size={28} className="text-muted" />
      <h1 className="text-lg font-semibold">Welcome to worki</h1>
      <p className="max-w-xs text-center text-xs leading-relaxed text-muted">
        Add a git repository, then spin up worktrees to give each agent its own isolated checkout.
      </p>
      <div className="flex gap-2">
        <Button variant="primary" onClick={() => addProject.mutate()}>
          Add repository…
        </Button>
        <Button onClick={() => openDialog({ kind: "addRemoteProject" })}>Add remote (SSH)…</Button>
      </div>
      {addProject.error && <p className="text-xs text-removed">{String(addProject.error)}</p>}
    </Centered>
  );
}

function EmptyProject({ onCreate }: { onCreate: () => void }) {
  return (
    <Centered>
      <GitBranchPlus size={28} className="text-muted" />
      <Button variant="primary" onClick={onCreate}>
        New worktree
      </Button>
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
