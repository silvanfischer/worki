import clsx from "clsx";
import { useState } from "react";
import { ArrowDown, ArrowUp, Coins, GitBranch, MoreHorizontal, PanelRight, Send } from "lucide-react";

import { type ChangedFile, errorMessage, type Project, type Worktree, worktreeLabel } from "../lib/api";
import { useBranchChanges, useCompareBase, useSessions, useWorktreeStatus } from "../lib/queries";
import { reviewPrompt, reviewTarget, sendReview, useReview, useWorktreeComments } from "../lib/review";
import { useUi } from "../lib/store";
import { formatTokens, totalUsage, usageDetails } from "../lib/usage";
import { BranchChip, BranchCommits, BranchStatus, CommitBox, PrSection, Section } from "./BranchPanel";
import { DiffView } from "./DiffView";
import { OpenInMenu } from "./OpenInMenu";
import { ResizeHandle } from "./ResizeHandle";
import { SessionArea } from "./SessionArea";
import { Button, DiffStat, ErrorText, IconButton, Menu, Select } from "./ui";
import { WorktreeIsolationBadge } from "./WorktreeIsolation";
import { worktreeActions } from "./worktreeActions";

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
  const base = useCompareBase(worktree.path, worktree.isMain ? null : project.defaultBase);
  const showCommits = useUi((s) => s.showCommits);
  const showPullRequests = useUi((s) => s.showPullRequests);
  const hasBranchTab = !worktree.isMain && !!base;
  const tab = scope === "branch" && hasBranchTab ? "branch" : "uncommitted";
  const branchScope = tab === "branch";
  const branch = useBranchChanges(worktree.path, base);
  const pending = comments.length;
  const [excluded, setExcluded] = useState<Set<string>>(new Set());

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
        {base && (showCommits || showPullRequests) && (
          <BranchChip
            worktree={worktree}
            base={base}
            onOpen={() => {
              setScope("branch");
              if (!showChanges) toggleChanges();
            }}
          />
        )}
        <WorktreeIsolationBadge project={project} worktree={worktree} />
        <OpenInMenu path={worktree.path} />
        <Menu
          trigger={(toggle) => (
            <IconButton label="More actions" onClick={toggle}>
              <MoreHorizontal size={15} />
            </IconButton>
          )}
          items={worktreeActions(project, worktree, openDialog)}
        />
        <IconButton
          label={`${showChanges ? "Hide" : "Show"} changes (⌘J)${pending > 0 ? ` · ${pending} review comment${pending === 1 ? "" : "s"}` : ""}`}
          onClick={toggleChanges}
          className={clsx("relative", showChanges && "text-fg")}
        >
          <PanelRight size={14} />
          {pending > 0 && (
            <span className="absolute -top-0.5 -right-0.5 flex size-3.5 items-center justify-center rounded-full bg-accent text-[9px] font-semibold text-accent-fg">
              {pending}
            </span>
          )}
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
              key={`${diffFile}:${branchScope}`}
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
            <div className="flex h-9 shrink-0 items-center gap-1 border-b border-line px-2">
              {(["uncommitted", "branch"] as const)
                .filter((t) => t === "uncommitted" || hasBranchTab)
                .map((t) => (
                  <button
                    key={t}
                    onClick={() => setScope(t)}
                    title={
                      t === "uncommitted"
                        ? "What you haven't committed yet"
                        : `This branch compared with ${base}: commits, pull request and all changed files`
                    }
                    className={clsx(
                      "flex h-7 items-center gap-1.5 rounded-md px-2 text-xs",
                      tab === t ? "bg-selected font-medium text-fg" : "text-muted hover:bg-hover hover:text-fg",
                    )}
                  >
                    {t === "uncommitted" ? "Changes" : "Branch"}
                    {t === "uncommitted" && !!status?.changedFiles.length && (
                      <span className="text-[11px] text-faint" title="Uncommitted files">
                        {status.changedFiles.length}
                      </span>
                    )}
                  </button>
                ))}
            </div>

            <div className="selectable min-h-0 flex-1 overflow-y-auto">
              {tab === "uncommitted" && (
                <>
                  <FileList
                    files={status?.changedFiles}
                    error={error}
                    empty="No uncommitted changes."
                    excluded={showCommits ? excluded : undefined}
                    onExclude={setExcluded}
                    comments={comments}
                    diffFile={diffFile}
                    onOpen={setDiffFile}
                  />
                  {showCommits && !!status?.changedFiles.length && (
                    <CommitBox
                      worktreePath={worktree.path}
                      files={status.changedFiles.map((f) => f.path).filter((p) => !excluded.has(p))}
                      total={status.changedFiles.length}
                    />
                  )}
                </>
              )}

              {tab === "branch" && !worktree.isMain && base && (
                <>
                  {showCommits && worktree.branch && branch.data && (
                    <BranchStatus
                      worktreePath={worktree.path}
                      branch={worktree.branch}
                      base={base}
                      ahead={branch.data.commits}
                      behind={branch.data.behind}
                    />
                  )}
                  {showCommits && <BranchCommits worktreePath={worktree.path} base={base} />}
                  {showPullRequests && worktree.branch && <PrSection worktree={worktree} base={base} />}
                  <Section
                    title={`Files changed vs ${base}${branch.data ? ` (${branch.data.files.length})` : ""}`}
                    aside={branch.data && <DiffStat insertions={branch.data.insertions} deletions={branch.data.deletions} />}
                  >
                    <FileList
                      files={branch.data?.files}
                      error={branch.error}
                      empty={`Nothing changed since ${base}.`}
                      comments={comments}
                      diffFile={diffFile}
                      onOpen={setDiffFile}
                    />
                  </Section>
                </>
              )}
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
  const [picked, setPicked] = useState<string | null>(null);
  if (comments.length === 0)
    return <p className="border-t border-line px-3 py-2 text-[11px] text-faint">Hover a line in a diff to comment on it.</p>;

  const agents = sessions.filter((s) => s.worktreePath === worktreePath && s.profileId && s.state.kind === "running");
  const target = agents.find((s) => s.id === picked) ?? reviewTarget(agents, activeId);
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
      {agents.length > 1 && target && (
        <label className="flex items-center gap-2 text-[11px] text-muted">
          To
          <Select
            value={target.id}
            onChange={(e) => setPicked(e.target.value)}
            className="h-7 min-w-0 flex-1 text-xs"
          >
            {agents.map((a, i) => (
              <option key={a.id} value={a.id}>
                {a.title} {i + 1}
                {a.id === activeId ? " (open tab)" : ""}
              </option>
            ))}
          </Select>
        </label>
      )}
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

function FileList({
  files,
  error,
  empty,
  excluded,
  onExclude,
  comments,
  diffFile,
  onOpen,
}: {
  files: ChangedFile[] | undefined;
  error: unknown;
  empty: string;
  /** Files left out of the next commit; undefined hides the checkboxes. */
  excluded?: Set<string>;
  onExclude?: (update: (ex: Set<string>) => Set<string>) => void;
  comments: { file: string }[];
  diffFile: string | null;
  onOpen: (file: string | null) => void;
}) {
  return (
    <div className="px-1.5 py-1">
      {!!error && <p className="px-2 text-xs text-removed">{String(error)}</p>}
      {files?.length === 0 && <p className="px-2 py-1 text-xs text-faint">{empty}</p>}
      {files?.map((f) => (
        <ChangeRow
          key={f.path}
          file={f}
          checked={excluded ? !excluded.has(f.path) : undefined}
          onCheck={(on) =>
            onExclude?.((ex) => {
              const next = new Set(ex);
              if (on) next.delete(f.path);
              else next.add(f.path);
              return next;
            })
          }
          comments={comments.filter((c) => c.file === f.path).length}
          selected={f.path === diffFile}
          onOpen={() => onOpen(f.path === diffFile ? null : f.path)}
        />
      ))}
    </div>
  );
}

function ChangeRow({
  file,
  checked,
  onCheck,
  comments,
  selected,
  onOpen,
}: {
  file: ChangedFile;
  /** Whether the file goes into the next commit; undefined hides the checkbox. */
  checked?: boolean;
  onCheck: (on: boolean) => void;
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
      {checked !== undefined && (
        <input
          type="checkbox"
          checked={checked}
          onClick={(e) => e.stopPropagation()}
          onChange={(e) => onCheck(e.target.checked)}
          title={checked ? "Included in the next commit" : "Left out of the next commit"}
          className="size-3 shrink-0 accent-[var(--accent)]"
        />
      )}
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
