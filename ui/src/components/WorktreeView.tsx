import { revealItemInDir } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import { useState } from "react";
import { ArrowDown, ArrowUp, Coins, Copy, FolderOpen, GitBranch, PanelRight, Send, Trash2 } from "lucide-react";

import { type ChangedFile, errorMessage, type Project, type Worktree, worktreeLabel } from "../lib/api";
import { useBranchChanges, useSessions, useWorktreeStatus } from "../lib/queries";
import { reviewPrompt, reviewTarget, sendReview, useReview, useWorktreeComments } from "../lib/review";
import { useUi } from "../lib/store";
import { formatTokens, totalUsage, usageDetails } from "../lib/usage";
import { DiffView } from "./DiffView";
import { OpenInMenu } from "./OpenInMenu";
import { ResizeHandle } from "./ResizeHandle";
import { SessionArea } from "./SessionArea";
import { Button, DiffStat, ErrorText, IconButton } from "./ui";
import { WorktreeIsolationBadge } from "./WorktreeIsolation";

export function WorktreeView({ project, worktree }: { project: Project; worktree: Worktree }) {
  const openDialog = useUi((s) => s.openDialog);
  const showChanges = useUi((s) => s.showChanges);
  const toggleChanges = useUi((s) => s.toggleChanges);
  const { data: status, error } = useWorktreeStatus(worktree.path, 5_000);
  const [diffFile, setDiffFile] = useState<string | null>(null);
  const widths = useUi((s) => s.panelWidths);
  const comments = useWorktreeComments(worktree.path);
  const scope = useUi((s) => s.changesScope);
  const setScope = useUi((s) => s.setChangesScope);
  const base = worktree.isMain ? null : project.defaultBase;
  const branchScope = scope === "branch" && !!base;
  const branch = useBranchChanges(worktree.path, branchScope ? base : null);
  const files = branchScope ? branch.data?.files : status?.changedFiles;
  const stats = branchScope ? branch.data : status;
  const listError = branchScope ? branch.error : error;

  return (
    <div className="flex h-full flex-col">
      <header data-tauri-drag-region className="flex h-11 shrink-0 items-center gap-3 border-b border-line px-4">
        <GitBranch size={15} className="text-muted" />
        <div data-tauri-drag-region className="flex min-w-0 flex-1 items-baseline gap-2">
          <h1 className="truncate text-[14px] font-semibold">{worktreeLabel(worktree)}</h1>
          <span className="truncate text-xs text-faint">{project.name}</span>
        </div>
        {status && (status.ahead > 0 || status.behind > 0) && (
          <span className="flex items-center gap-1.5 font-mono text-xs text-muted" title={`vs ${status.upstream}`}>
            <ArrowUp size={12} />
            {status.ahead}
            <ArrowDown size={12} />
            {status.behind}
          </span>
        )}
        <WorktreeUsage worktreePath={worktree.path} />
        <WorktreeIsolationBadge project={project} worktree={worktree} />
        <OpenInMenu path={worktree.path} />
        <IconButton label="Copy path" onClick={() => navigator.clipboard.writeText(worktree.path)}>
          <Copy size={14} />
        </IconButton>
        <IconButton label="Reveal in Finder" onClick={() => revealItemInDir(worktree.path)}>
          <FolderOpen size={14} />
        </IconButton>
        {!worktree.isMain && (
          <IconButton
            label="Remove worktree"
            onClick={() => openDialog({ kind: "removeWorktree", projectId: project.id, path: worktree.path })}
          >
            <Trash2 size={14} />
          </IconButton>
        )}
        <IconButton
          label={showChanges ? "Hide changes" : "Show changes"}
          onClick={toggleChanges}
          className={clsx(showChanges && "text-fg")}
        >
          <PanelRight size={14} />
        </IconButton>
      </header>

      <div className="flex min-h-0 flex-1 overflow-hidden">
        <section className="min-h-0 min-w-[240px] flex-1">
          <SessionArea project={project} worktree={worktree} />
        </section>

        {diffFile && (
          <section className="relative min-h-0 shrink-0 border-l border-line" style={{ width: widths.diff }}>
            <ResizeHandle panel="diff" edge="left" />
            <DiffView
              worktreePath={worktree.path}
              file={diffFile}
              base={branchScope ? base : null}
              onClose={() => setDiffFile(null)}
            />
          </section>
        )}

        {showChanges && (
          <aside className="relative flex min-h-0 shrink-0 flex-col border-l border-line" style={{ width: widths.changes }}>
            <ResizeHandle panel="changes" edge="left" />
            <div className="flex h-9 shrink-0 items-center justify-between gap-2 px-3">
              {base ? (
                <div className="flex rounded-md bg-hover p-0.5 text-[11px]">
                  {(["uncommitted", "branch"] as const).map((sc) => (
                    <button
                      key={sc}
                      onClick={() => setScope(sc)}
                      title={sc === "branch" ? `Everything this worktree changed since it left ${base}` : "Changes not committed yet"}
                      className={clsx(
                        "truncate rounded px-2 py-0.5",
                        scope === sc ? "bg-elevated text-fg shadow-xs" : "text-muted hover:text-fg",
                      )}
                    >
                      {sc === "branch" ? `vs ${base}` : "Uncommitted"}
                    </button>
                  ))}
                </div>
              ) : (
                <span className="text-[11px] font-semibold tracking-wide text-faint uppercase">Changes</span>
              )}
              {stats && <DiffStat insertions={stats.insertions} deletions={stats.deletions} />}
            </div>
            {branchScope && branch.data && (
              <p className="px-3 pb-1 text-[11px] text-faint">
                {branch.data.commits === 0
                  ? "No commits yet"
                  : `${branch.data.commits} commit${branch.data.commits === 1 ? "" : "s"}`}{" "}
                since {base}, plus uncommitted work
              </p>
            )}
            <div className="selectable flex-1 overflow-y-auto px-1.5 pb-3">
              {listError && <p className="px-2 text-xs text-removed">{String(listError)}</p>}
              {files?.length === 0 && (
                <p className="px-2 py-1 text-xs text-faint">{branchScope ? `Nothing changed since ${base}` : "Working tree clean"}</p>
              )}
              {files?.map((f) => (
                <ChangeRow
                  key={f.path}
                  file={f}
                  comments={comments.filter((c) => c.file === f.path).length}
                  selected={f.path === diffFile}
                  onOpen={() => setDiffFile(f.path === diffFile ? null : f.path)}
                />
              ))}
            </div>
            <ReviewBar worktreePath={worktree.path} onOpen={setDiffFile} />
          </aside>
        )}
      </div>
    </div>
  );
}

function WorktreeUsage({ worktreePath }: { worktreePath: string }) {
  const { data: sessions = [] } = useSessions();
  const total = totalUsage(sessions.filter((s) => s.worktreePath === worktreePath).flatMap((s) => s.usage));
  if (!total) return null;
  return (
    <span
      className="flex items-center gap-1 font-mono text-xs text-muted"
      title={`Tokens used by agents in this worktree (open sessions)\n${usageDetails(total)}`}
    >
      <Coins size={12} />
      {formatTokens(total.tokens)} tokens
    </span>
  );
}

function ReviewBar({ worktreePath, onOpen }: { worktreePath: string; onOpen: (file: string) => void }) {
  const comments = useWorktreeComments(worktreePath);
  const clear = useReview((s) => s.clear);
  const { data: sessions = [] } = useSessions();
  const activeId = useUi((s) => s.activeSession[worktreePath]);
  const setActive = useUi((s) => s.setActiveSession);
  const [error, setError] = useState<string | null>(null);
  if (comments.length === 0)
    return <p className="border-t border-line px-3 py-2 text-[11px] text-faint">Hover a line in a diff to comment on it.</p>;

  const target = reviewTarget(
    sessions.filter((s) => s.worktreePath === worktreePath),
    activeId,
  );
  const files = [...new Set(comments.map((c) => c.file))];

  async function send() {
    if (!target) return;
    setError(null);
    try {
      await sendReview(target.id, reviewPrompt(comments));
      setActive(worktreePath, target.id);
      clear(worktreePath);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <div className="flex flex-col gap-2 border-t border-line p-3">
      <div className="flex items-baseline justify-between">
        <span className="text-xs font-medium">
          {comments.length} comment{comments.length === 1 ? "" : "s"}
        </span>
        <button className="text-[11px] text-muted hover:text-fg" onClick={() => clear(worktreePath)}>
          Discard
        </button>
      </div>
      <div className="flex flex-col">
        {files.map((f) => (
          <button key={f} onClick={() => onOpen(f)} className="truncate text-left font-mono text-[11px] text-muted hover:text-fg">
            {f}
          </button>
        ))}
      </div>
      <Button
        variant="primary"
        onClick={send}
        disabled={!target}
        title={target ? `Paste the review into ${target.title} and submit it` : undefined}
      >
        <Send size={12} />
        {target ? `Send to ${target.title}` : "Send to agent"}
      </Button>
      {!target && <p className="text-[11px] text-faint">Start an agent in this worktree to send the review.</p>}
      <ErrorText>{error}</ErrorText>
    </div>
  );
}

function ChangeRow({
  file,
  comments,
  selected,
  onOpen,
}: {
  file: ChangedFile;
  comments: number;
  selected: boolean;
  onOpen: () => void;
}) {
  const kind = changeKind(file.code);
  const slash = file.path.lastIndexOf("/");
  return (
    <div
      onClick={onOpen}
      className={clsx("flex h-6 items-center gap-2 rounded px-1.5", selected ? "bg-selected" : "hover:bg-hover")}
      title={`${file.path} — click to view the diff`}
    >
      <span
        className={clsx(
          "w-3 text-center font-mono text-[11px] font-semibold",
          kind === "A" || kind === "U" ? "text-added" : kind === "D" ? "text-removed" : "text-warn",
        )}
      >
        {kind}
      </span>
      <span className="min-w-0 truncate text-xs">
        {file.path.slice(slash + 1)}
        {slash > 0 && <span className="ml-1.5 text-faint">{file.path.slice(0, slash)}</span>}
      </span>
      {comments > 0 && (
        <span className="ml-auto rounded-full bg-accent px-1.5 text-[10px] text-accent-fg" title={`${comments} review comments`}>
          {comments}
        </span>
      )}
    </div>
  );
}

function changeKind(code: string): string {
  if (code === "??") return "U";
  const c = code.trim()[0] ?? "M";
  return c === "R" || c === "C" ? "R" : c;
}
