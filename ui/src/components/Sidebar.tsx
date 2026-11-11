import { useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import {
  ArrowDownToLine,
  BellRing,
  ChevronRight,
  X,
  MessageSquare,
  FolderGit2,
  GitBranch,
  House,
  LoaderCircle,
  MoreHorizontal,
  Plus,
  Server,
  Settings,
  Settings2,
} from "lucide-react";

import { api, type Project, type Worktree, worktreeLabel } from "../lib/api";
import { useWorktreeComments } from "../lib/review";
import { keys, useAddProject, useProjects, useSessions, useWorktrees, useWorktreeStatus } from "../lib/queries";
import { useUi } from "../lib/store";
import { installUpdate, useUpdates } from "../lib/updates";
import { ActivityDot, summarize } from "./ActivityDot";
import { ResizeHandle } from "./ResizeHandle";
import { DiffStat, ErrorText, IconButton, Menu } from "./ui";
import { worktreeActions } from "./worktreeActions";

export function Sidebar() {
  const { data: projects = [] } = useProjects();
  const addProject = useAddProject();
  const openDialog = useUi((s) => s.openDialog);
  const width = useUi((s) => s.panelWidths.sidebar);

  return (
    <aside className="relative flex h-full shrink-0 flex-col" style={{ width }}>
      <ResizeHandle panel="sidebar" edge="right" />
      <div data-tauri-drag-region className="h-11 shrink-0" />
      <div className="flex items-center justify-between px-3 pb-1">
        <span className="text-[11px] font-semibold tracking-wide text-faint uppercase">Projects</span>
        <Menu
          trigger={(toggle) => (
            <IconButton label="Add project" onClick={toggle}>
              <Plus size={15} />
            </IconButton>
          )}
          items={[
            { label: "Add local repository…", icon: <FolderGit2 size={13} />, onSelect: () => addProject.mutate() },
            {
              label: "Add remote repository (SSH)…",
              icon: <Server size={13} />,
              onSelect: () => openDialog({ kind: "addRemoteProject" }),
            },
          ]}
        />
      </div>
      {addProject.error && (
        <div className="px-3 pb-2">
          <ErrorText>{String(addProject.error)}</ErrorText>
        </div>
      )}
      <nav className="flex-1 overflow-y-auto px-2 pb-3">
        <NeedsYou projects={projects} />
        {projects.map((p) => (
          <ProjectNode key={p.id} project={p} />
        ))}
        {projects.length === 0 && (
          <p className="px-2 py-4 text-xs leading-relaxed text-faint">
            No projects yet. Add a git repository to start managing its worktrees.
          </p>
        )}
      </nav>
      <div className="flex shrink-0 items-center border-t border-line px-2 py-1.5">
        <button
          onClick={() => openDialog({ kind: "settings", tab: "general" })}
          title="Settings (⌘,)"
          className="flex h-7 items-center gap-2 rounded-md px-2 text-muted hover:bg-hover hover:text-fg"
        >
          <Settings size={14} />
          Settings
        </button>
        <UpdateButton />
      </div>
    </aside>
  );
}

function ProjectNode({ project }: { project: Project }) {
  const collapsed = useUi((s) => !!s.collapsed[project.id]);
  const toggle = useUi((s) => s.toggleCollapsed);
  const openDialog = useUi((s) => s.openDialog);
  const { data: worktrees = [], error } = useWorktrees(project.id);

  return (
    <div className="mb-1">
      <div className="group flex h-7 items-center gap-1 rounded-md pr-1 pl-1 hover:bg-hover">
        <button className="flex min-w-0 flex-1 items-center gap-1" onClick={() => toggle(project.id)}>
          <ChevronRight size={13} className={clsx("shrink-0 text-faint transition-transform", !collapsed && "rotate-90")} />
          {project.remote ? (
            <Server size={14} className="shrink-0 text-muted" />
          ) : (
            <FolderGit2 size={14} className="shrink-0 text-muted" />
          )}
          <span className="truncate font-semibold" title={project.remote ? `${project.remote.host}:${project.repoPath}` : project.repoPath}>
            {project.name}
          </span>
        </button>
        <div className="flex opacity-0 group-hover:opacity-100">
          <IconButton label="New worktree" onClick={() => openDialog({ kind: "newWorktree", projectId: project.id })}>
            <Plus size={14} />
          </IconButton>
          <IconButton label="Project settings" onClick={() => openDialog({ kind: "projectSettings", projectId: project.id })}>
            <Settings2 size={13} />
          </IconButton>
        </div>
      </div>
      {!collapsed && (
        <div className="mt-0.5 flex flex-col gap-px">
          {error && <p className="px-7 py-1 text-xs text-removed">{String(error)}</p>}
          {worktrees.map((wt) => (
            <WorktreeRow key={wt.path} project={project} worktree={wt} />
          ))}
          <button
            onClick={() => openDialog({ kind: "newWorktree", projectId: project.id })}
            className="flex h-7 items-center gap-2 rounded-md pl-7 text-left text-muted hover:bg-hover hover:text-fg"
          >
            <Plus size={13} />
            New worktree
          </button>
        </div>
      )}
    </div>
  );
}

function WorktreeRow({ project, worktree }: { project: Project; worktree: Worktree }) {
  const selected = useUi((s) => s.selectedWorktreePath === worktree.path);
  const select = useUi((s) => s.select);
  const openDialog = useUi((s) => s.openDialog);
  const qc = useQueryClient();
  const { data: status } = useWorktreeStatus(worktree.path, 30_000);
  const { data: sessions = [] } = useSessions();
  const dirty = status?.changedFiles.length ?? 0;
  const mine = sessions.filter((s) => s.worktreePath === worktree.path);
  const activity = summarize(mine);
  const waitingOn = mine.find((s) => s.state.kind === "running" && s.activity === "waiting");
  const comments = useWorktreeComments(worktree.path).length;

  return (
    <div
      onClick={() => {
        select(project.id, worktree.path);
        qc.invalidateQueries({ queryKey: keys.status(worktree.path) });
      }}
      className={clsx(
        "group flex h-7 items-center gap-2 rounded-md pr-1 pl-7",
        selected ? "bg-selected" : "hover:bg-hover",
      )}
    >
      {worktree.isMain ? (
        <House size={13} className="shrink-0 text-faint" />
      ) : (
        <GitBranch size={13} className="shrink-0 text-faint" />
      )}
      <span className={clsx("min-w-0 flex-1 truncate", worktree.prunable && "text-faint line-through")}>
        {worktreeLabel(worktree)}
      </span>
      {activity && activity !== "exited" && (
        <ActivityDot
          state={activity}
          title={waitingOn ? `${waitingOn.title} is waiting${waitingOn.attention ? `: ${waitingOn.attention}` : ""}` : undefined}
        />
      )}
      {comments > 0 && (
        <span
          className="flex items-center gap-0.5 text-[11px] text-accent group-hover:hidden"
          title={`${comments} review comment${comments === 1 ? "" : "s"} not sent yet`}
        >
          <MessageSquare size={11} />
          {comments}
        </span>
      )}
      {status && (status.insertions > 0 || status.deletions > 0) ? (
        <DiffStat insertions={status.insertions} deletions={status.deletions} className="group-hover:hidden" />
      ) : dirty > 0 ? (
        <span className="size-1.5 rounded-full bg-warn group-hover:hidden" title={`${dirty} changed files`} />
      ) : null}
      <div className="hidden group-hover:block" onClick={(e) => e.stopPropagation()}>
        <Menu
          trigger={(toggle) => (
            <IconButton label="Worktree actions" onClick={toggle} className="size-5">
              <MoreHorizontal size={14} />
            </IconButton>
          )}
          items={worktreeActions(project, worktree, openDialog)}
        />
      </div>
    </div>
  );
}

/** Agents waiting for an answer, across all projects, so none of them sits unnoticed. */
function NeedsYou({ projects }: { projects: Project[] }) {
  const { data: sessions = [] } = useSessions();
  const select = useUi((s) => s.select);
  const setActive = useUi((s) => s.setActiveSession);
  const waiting = sessions.filter((s) => s.state.kind === "running" && s.activity === "waiting");
  if (waiting.length === 0) return null;
  return (
    <div className="mb-2 rounded-lg bg-warn/8 p-1">
      <div className="flex items-center gap-1.5 px-2 pt-1 pb-1 text-[11px] font-semibold tracking-wide text-warn uppercase">
        <BellRing size={11} />
        Needs you
      </div>
      {waiting.map((s) => {
        const project = projects.find((p) => s.worktreePath.startsWith(p.worktreeRoot) || s.worktreePath === p.repoPath);
        const name = s.worktreePath.split("/").pop();
        return (
          <div key={s.id} className="group/need flex items-start rounded-md hover:bg-hover">
            <button
              onClick={() => {
                if (project) select(project.id, s.worktreePath);
                setActive(s.worktreePath, s.id);
              }}
              title={s.attention ?? `${s.title} is waiting for you`}
              className="flex min-w-0 flex-1 flex-col px-2 py-1 text-left"
            >
              <span className="truncate text-[13px]">
                {s.title} <span className="text-faint">· {name}</span>
              </span>
              {s.attention && <span className="truncate text-[11px] text-muted">{s.attention}</span>}
            </button>
            <IconButton
              label="Skip for now: hides this without answering"
              onClick={() => api.dismissAttention(s.id).catch(() => {})}
              className="mt-1 mr-1 size-5 opacity-0 group-hover/need:opacity-100"
            >
              <X size={12} />
            </IconButton>
          </div>
        );
      })}
    </div>
  );
}

/** Offers the newer release once one was found, and installs it on click. */
function UpdateButton() {
  const { available, status, progress, error } = useUpdates();
  if (!available) return null;
  const installing = status === "installing";
  return (
    <button
      onClick={installUpdate}
      disabled={installing}
      title={
        error
          ? `The update failed: ${error}`
          : `worki ${available.version} is available. Installs it and restarts worki; agents in sandboxes and containers keep running.`
      }
      className="ml-auto flex h-7 items-center gap-1.5 rounded-md bg-accent px-2 text-[12px] font-medium text-accent-fg hover:brightness-110 disabled:opacity-80"
    >
      {installing ? <LoaderCircle size={12} className="animate-spin" /> : <ArrowDownToLine size={12} />}
      {installing
        ? progress !== null
          ? `Updating ${Math.round(progress * 100)}%`
          : "Updating…"
        : error
          ? "Retry update"
          : `Update to ${available.version}`}
    </button>
  );
}
