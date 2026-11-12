import { useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import {
  ArrowDownToLine,
  BellRing,
  CircleX,
  ChevronRight,
  X,
  MessageSquare,
  FolderGit2,
  GitBranch,
  House,
  LoaderCircle,
  Pencil,
  Plus,
  RefreshCw,
  Server,
  Settings,
  Settings2,
} from "lucide-react";
import { useState } from "react";

import { api, errorMessage, type Project, type PullRequest, type Worktree, worktreeLabel } from "../lib/api";
import { identityOf, isAgent, useJustChanged, usePlayful } from "../lib/playful";
import { useWorktreeComments } from "../lib/review";
import { keys, useAddProject, useProjects, usePullRequest, useSessions, useWorktrees, useWorktreeStatus } from "../lib/queries";
import { useUi } from "../lib/store";
import { plural } from "../lib/text";
import { toast } from "../lib/toast";
import { installUpdate, useUpdates } from "../lib/updates";
import { ActivityDot, summarize } from "./ActivityDot";
import { PrStateIcon } from "./BranchPanel";
import { AgentDot } from "./Playful";
import { ResizeHandle } from "./ResizeHandle";
import { ContextMenu, ErrorText, Hint, IconButton, Menu } from "./ui";
import { worktreeActions } from "./worktreeActions";

export function Sidebar() {
  const { data: projects = [] } = useProjects();
  const addProject = useAddProject();
  const openDialog = useUi((s) => s.openDialog);
  const width = useUi((s) => s.panelWidths.sidebar);

  return (
    <aside className="fs-sidebar relative flex h-full shrink-0 flex-col bg-sidebar" style={{ width }}>
      <ResizeHandle panel="sidebar" edge="right" />
      <div data-tauri-drag-region className="h-13 shrink-0" />
      <div className="flex items-center justify-between px-3 pb-1">
        <span className="fs-sidebar-xs font-semibold tracking-wide text-faint uppercase">Projects</span>
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
          <p className="px-2 py-4 fs-sidebar-xs leading-relaxed text-faint">
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
  const qc = useQueryClient();
  const [fetching, setFetching] = useState(false);

  async function fetchAll() {
    setFetching(true);
    try {
      await api.fetchRemote(project.repoPath);
      await qc.invalidateQueries({
        predicate: ({ queryKey: [kind] }) =>
          ["branches", "compareBase", "branchLog", "branchChanges", "status", "pullRequest"].includes(kind as string),
      });
    } catch (e) {
      toast(`Couldn't fetch ${project.name}: ${errorMessage(e)}`);
    } finally {
      setFetching(false);
    }
  }

  return (
    <div className="mb-1">
      <div className="group flex h-7 items-center gap-1 rounded-md pr-1 pl-1 hover:bg-hover">
        <button className="flex min-w-0 flex-1 items-center gap-1" onClick={() => toggle(project.id)}>
          <ChevronRight size={13} className={clsx("shrink-0 text-faint transition-transform", !collapsed && "rotate-90")} />
          {project.remote ? (
            <Hint title={`Remote repository on ${project.remote.host}`} className="text-muted">
              <Server size={14} />
            </Hint>
          ) : (
            <Hint title="Repository on this Mac" className="text-muted">
              <FolderGit2 size={14} />
            </Hint>
          )}
          <span className="truncate font-semibold" title={project.remote ? `${project.remote.host}:${project.repoPath}` : project.repoPath}>
            {project.name}
          </span>
        </button>
        <div className="flex opacity-0 group-hover:opacity-100 has-disabled:opacity-100">
          <IconButton label="Fetch (git fetch --all)" onClick={fetchAll} disabled={fetching}>
            <RefreshCw size={13} className={clsx(fetching && "animate-spin")} />
          </IconButton>
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
          {error && <p className="px-7 py-1 fs-sidebar-xs text-removed">{String(error)}</p>}
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
  const showPullRequests = useUi((s) => s.showPullRequests);
  const playful = usePlayful();
  const qc = useQueryClient();
  const { data: status } = useWorktreeStatus(worktree.path, 30_000);
  const { data: sessions = [] } = useSessions();
  const { data: pr } = usePullRequest(worktree.path, showPullRequests && !worktree.isMain && !!worktree.branch);
  const comments = useWorktreeComments(worktree.path).length;
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);

  const dirty = status?.changedFiles.length ?? 0;
  const mine = sessions.filter((s) => s.worktreePath === worktree.path);
  const activity = summarize(mine);
  const waitingOn = mine.find((s) => s.state.kind === "running" && s.activity === "waiting");
  const agents = playful ? mine.filter((s) => isAgent(s) && s.state.kind === "running") : [];

  return (
    <div
      onClick={() => {
        select(project.id, worktree.path);
        qc.invalidateQueries({ queryKey: keys.status(worktree.path) });
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        setMenuAt({ x: e.clientX, y: e.clientY });
      }}
      title="Right-click for worktree actions"
      className={clsx("flex h-7 items-center gap-2 rounded-md pr-1 pl-7", selected ? "bg-selected" : "hover:bg-hover")}
    >
      <WorktreeIcon worktree={worktree} pr={pr} />
      <span className={clsx("min-w-0 flex-1 truncate", worktree.prunable && "text-faint line-through")}>
        {worktreeLabel(worktree)}
      </span>
      {dirty > 0 && (
        <Hint title={plural(dirty, "uncommitted file")} className="text-warn">
          <Pencil size={11} />
        </Hint>
      )}
      {pr && checksFailed(pr) && (
        <Hint title={`${plural(pr.checksFailed, "check")} failed on #${pr.number}`} className="text-removed">
          <CircleX size={12} />
        </Hint>
      )}
      {agents.length > 0 ? (
        <span className="flex shrink-0 gap-0.5">
          {agents.slice(0, 3).map((s) => (
            <AgentDot key={s.id} session={s} />
          ))}
        </span>
      ) : (
        activity &&
        activity !== "exited" && (
          <ActivityDot
            state={activity}
            title={waitingOn ? `${waitingOn.title} is waiting${waitingOn.attention ? `: ${waitingOn.attention}` : ""}` : undefined}
          />
        )
      )}
      {comments > 0 && (
        <span
          className="flex items-center gap-0.5 fs-sidebar-xs text-accent"
          title={`${plural(comments, "review comment")} not sent yet`}
        >
          <MessageSquare size={11} />
          {comments}
        </span>
      )}
      {menuAt && (
        <ContextMenu at={menuAt} items={worktreeActions(project, worktree, openDialog)} onClose={() => setMenuAt(null)} />
      )}
    </div>
  );
}

/** The main checkout's house, the pull request's state, or a plain branch. */
function WorktreeIcon({ worktree, pr }: { worktree: Worktree; pr: PullRequest | null | undefined }) {
  if (worktree.isMain) {
    return (
      <Hint title="Main checkout: the repository folder itself" className="text-faint">
        <House size={13} />
      </Hint>
    );
  }
  if (!pr) {
    return (
      <Hint title={`Worktree on branch ${worktree.branch ?? "(detached)"}, no pull request yet`} className="text-faint">
        <GitBranch size={13} />
      </Hint>
    );
  }
  return (
    <Hint title={`Pull request #${pr.number} ${prStateLabel(pr)}: ${pr.title}`}>
      <PrStateIcon pr={pr} size={13} />
    </Hint>
  );
}

/** Agents waiting for an answer, across all projects, so none of them sits unnoticed. */
function NeedsYou({ projects }: { projects: Project[] }) {
  const { data: sessions = [] } = useSessions();
  const select = useUi((s) => s.select);
  const setActive = useUi((s) => s.setActiveSession);
  const playful = usePlayful();

  const waiting = sessions.filter((s) => s.state.kind === "running" && s.activity === "waiting");
  const allQuiet = useJustChanged(waiting.length > 0, true, false, 4000);

  if (waiting.length === 0) {
    if (!playful || !allQuiet) return null;
    return (
      <div className="playful-fade mb-2 rounded-lg bg-added/8 px-3 py-2 fs-sidebar-xs text-added">
        🌿 All quiet. Your agents are on it.
      </div>
    );
  }
  return (
    <div className="mb-2 rounded-lg bg-warn/8 p-1">
      <div
        className="flex items-center gap-1.5 px-2 pt-1 pb-1 fs-sidebar-xs font-semibold tracking-wide text-warn uppercase"
        title="Agents waiting for an answer, across all projects"
      >
        <BellRing size={11} />
        Needs you
      </div>
      {waiting.map((s) => {
        const project = projects.find((p) => s.worktreePath.startsWith(p.worktreeRoot) || s.worktreePath === p.repoPath);
        const identity = identityOf(s, playful);
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
              <span className="flex min-w-0 items-center gap-1.5 fs-sidebar">
                {identity && <AgentDot session={s} />}
                <span className="truncate">
                  {identity?.name ?? s.title} <span className="text-faint">· {s.worktreePath.split("/").pop()}</span>
                </span>
              </span>
              {s.attention && <span className="truncate fs-sidebar-xs text-muted">{s.attention}</span>}
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

const checksFailed = (pr: PullRequest) => pr.state === "OPEN" && pr.checksFailed > 0;

function prStateLabel(pr: PullRequest) {
  if (pr.state === "MERGED") return "merged";
  if (pr.state === "CLOSED") return "closed";
  return pr.isDraft ? "draft" : "open";
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
