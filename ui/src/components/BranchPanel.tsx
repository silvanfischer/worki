import { useQueryClient } from "@tanstack/react-query";
import { openUrl } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import {
  ArrowUpFromLine,
  Bot,
  CircleCheck,
  CircleDot,
  CircleX,
  ExternalLink,
  GitCommitHorizontal,
  GitMerge,
  GitPullRequest,
  LoaderCircle,
  MessageSquare,
  RefreshCw,
} from "lucide-react";
import { type ReactNode, useState } from "react";

import { api, type CommitInfo, errorMessage, type PullRequest, type SessionInfo, type Worktree } from "../lib/api";
import {
  keys,
  useAgentSettings,
  useBranchLog,
  useProjects,
  usePullRequest,
  useSessions,
  useWorktreeStatus,
} from "../lib/queries";
import { askPrompt } from "../lib/askPrompts";
import { reviewTarget, sendReview, useReview } from "../lib/review";
import { useUi } from "../lib/store";
import { toast } from "../lib/toast";
import { Button, Checkbox, Dialog, ErrorText, Field, Input, Textarea } from "./ui";

/** Where "Ask agent" tasks go. With an agent picked for them in the settings, one helper tab per
 * worktree, started with the first task and kept apart from the agents the user works with;
 * otherwise the open agent tab (the newest running agent if the open tab isn't one). */
function useAgentTarget(worktreePath: string) {
  const qc = useQueryClient();
  const { data: sessions = [] } = useSessions();
  const { data: projects = [] } = useProjects();
  const { data: agents } = useAgentSettings();
  const activeId = useUi((s) => s.activeSession[worktreePath]);
  const helperId = useUi((s) => s.askSession[worktreePath]);
  const setActive = useUi((s) => s.setActiveSession);
  const setHelper = useUi((s) => s.setAskSession);
  const taskProfileId = useUi((s) => s.gitTaskProfile);
  const profile = agents?.profiles.find((p) => p.id === taskProfileId);
  const here = sessions.filter((s) => s.worktreePath === worktreePath);
  const helper = here.find((s) => s.id === helperId && s.state.kind === "running");
  const open = profile ? undefined : reviewTarget(here, activeId, helperId);
  const target: { title: string } | undefined = profile
    ? { title: helper ? "the helper tab" : `a new ${profile.name} helper tab` }
    : open;
  /** The agent that writes text worki fills in itself, such as a pull request description. */
  const writerProfileId = profile?.id ?? open?.profileId ?? null;
  const ask = async (prompt: string) => {
    try {
      const existing = helper ?? open;
      if (existing) {
        await sendReview(existing.id, prompt);
        setActive(worktreePath, existing.id);
      } else if (profile) {
        const project = projects.find(
          (p) => worktreePath === p.repoPath || worktreePath.startsWith(`${p.worktreeRoot}/`),
        );
        if (!project) return;
        const stale = here.find((s) => s.id === helperId);
        if (stale) await api.closeSession(stale.id).catch(() => {});
        const info = await api.spawnSession(project.id, worktreePath, profile.id, 120, 32, null, prompt);
        qc.setQueryData<SessionInfo[]>(keys.sessions, (old = []) => [
          ...old.filter((s) => s.id !== stale?.id && s.id !== info.id),
          info,
        ]);
        setHelper(worktreePath, info.id);
        setActive(worktreePath, info.id);
      }
    } catch (e) {
      toast(errorMessage(e));
    }
  };
  return { target, ask, writerProfileId };
}

function useRefreshGit(worktreePath: string) {
  const qc = useQueryClient();
  return () =>
    Promise.all(
      [keys.status(worktreePath), ["branchLog", worktreePath], ["branchChanges", worktreePath], keys.pullRequest(worktreePath)].map(
        (queryKey) => qc.invalidateQueries({ queryKey }),
      ),
    );
}

/** Message box and Commit button under the uncommitted files. */
export function CommitBox({ worktreePath, files, total }: { worktreePath: string; files: string[]; total: number }) {
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refresh = useRefreshGit(worktreePath);
  const { target, ask } = useAgentTarget(worktreePath);
  const blocked =
    files.length === 0 ? "Check at least one file to commit." : !message.trim() ? "Write a commit message to commit." : null;

  async function commit() {
    if (blocked) return;
    setBusy(true);
    setError(null);
    try {
      await api.commitChanges(worktreePath, files, message);
      setMessage("");
      await refresh();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-2 border-t border-line p-3">
      <Textarea
        rows={2}
        value={message}
        onChange={(e) => setMessage(e.target.value)}
        placeholder="Commit message"
        className="text-[13px]"
        onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && commit()}
      />
      <div className="flex gap-2">
        <Button
          variant="primary"
          className="flex-1"
          onClick={commit}
          disabled={busy || !!blocked}
          title={blocked ?? `Commit ${files.length} of ${total} uncommitted ${total === 1 ? "file" : "files"} (⌘↩)`}
        >
          {busy ? <LoaderCircle size={12} className="animate-spin" /> : <GitCommitHorizontal size={13} />}
          Commit{files.length < total && ` ${files.length}/${total}`}
        </Button>
        <Button
          onClick={() =>
            ask(askPrompt("commit"))
          }
          disabled={!target}
          title={target ? `Ask ${target.title} to commit the changes itself` : "Start an agent in this worktree first"}
        >
          <Bot size={13} />
          Ask agent
        </Button>
      </div>
      {blocked && !error && <p className="text-[11px] text-faint">{blocked}</p>}
      <ErrorText>{error}</ErrorText>
    </div>
  );
}

function ago(seconds: number): string {
  const s = Date.now() / 1000 - seconds;
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

/** A titled block of the changes panel. */
export function Section({ title, aside, children }: { title: ReactNode; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1.5 border-b border-line px-3 py-2.5">
      <div className="flex items-center justify-between gap-2">
        <h3 className="truncate text-[11px] font-semibold tracking-wide text-faint uppercase">{title}</h3>
        {aside}
      </div>
      {children}
    </section>
  );
}

/** How the branch stands against its base, and rebasing onto it. */
export function BranchStatus({
  worktreePath,
  branch,
  base,
  ahead,
  behind,
}: {
  worktreePath: string;
  branch: string;
  base: string;
  ahead: number;
  behind: number;
}) {
  const [busy, setBusy] = useState(false);
  const [conflicts, setConflicts] = useState<string[] | null>(null);
  const refresh = useRefreshGit(worktreePath);
  const { target, ask } = useAgentTarget(worktreePath);

  async function rebase() {
    setBusy(true);
    setConflicts(null);
    try {
      await api.fetchRemote(worktreePath).catch((e) => toast(`Couldn't fetch, rebasing on what's here: ${errorMessage(e)}`));
      const result = await api.updateFromBase(worktreePath, base);
      if (result.kind === "conflicts") setConflicts(result.files);
      else toast(result.kind === "updated" ? `Rebased on ${base}` : `Already up to date with ${base}`, "info");
      await refresh();
    } catch (e) {
      toast(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;
  return (
    <div className="flex flex-col gap-2 border-b border-line px-3 py-2.5">
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="truncate font-mono text-xs">{branch}</div>
          <div className="text-[11px] text-muted">
            {ahead === 0 && behind === 0
              ? `Same as ${base}`
              : [ahead > 0 && `${plural(ahead, "commit")} ahead`, behind > 0 && `${behind} behind`]
                  .filter(Boolean)
                  .join(" · ") + ` ${base}`}
          </div>
        </div>
        <Button
          onClick={rebase}
          disabled={busy}
          variant={behind > 0 ? "primary" : "secondary"}
          title={`Fetch, then replay this branch's commits on top of ${base}. Uncommitted changes are kept.`}
        >
          <RefreshCw size={12} className={clsx(busy && "animate-spin")} />
          {busy ? "Rebasing…" : "Rebase"}
        </Button>
      </div>
      {behind > 0 && !conflicts && (
        <p className="text-[11px] text-faint">
          {base} has {plural(behind, "new commit")}. Rebase to build on them before opening a pull request.
        </p>
      )}
      {conflicts && (
        <div className="flex flex-col gap-2 rounded-md bg-warn/10 p-2 text-xs">
          <span>
            Rebasing on {base} conflicts in {conflicts.join(", ")}. Nothing was changed.
          </span>
          <Button
            className="self-start"
            disabled={!target}
            onClick={() => {
              setConflicts(null);
              ask(askPrompt("resolveConflicts", { base, files: conflicts.join(", ") }));
            }}
          >
            <Bot size={13} />
            Ask agent to resolve
          </Button>
        </div>
      )}
    </div>
  );
}

function CommitRows({ commits }: { commits: CommitInfo[] }) {
  return (
    <div className="flex max-h-48 flex-col overflow-y-auto">
      {commits.map((c) => (
        <div key={c.sha} className="flex h-6 items-center gap-2 text-xs" title={`${c.sha.slice(0, 8)} · ${c.author}`}>
          <GitCommitHorizontal size={12} className="shrink-0 text-faint" />
          <span className="min-w-0 flex-1 truncate">{c.subject}</span>
          <span className="shrink-0 text-[11px] text-faint">{ago(c.time)}</span>
        </div>
      ))}
    </div>
  );
}

/** The branch's own commits since it left its base. */
export function BranchCommits({ worktreePath, base }: { worktreePath: string; base: string }) {
  const { data: commits = [] } = useBranchLog(worktreePath, base);
  return (
    <Section title={`Commits${commits.length ? ` (${commits.length})` : ""}`}>
      {commits.length === 0 ? (
        <p className="text-xs text-faint">None yet. Commit your work in the Changes tab.</p>
      ) : (
        <CommitRows commits={commits} />
      )}
    </Section>
  );
}

/** The branch's pull request, or a way to open one. */
export function PrSection({ worktree, base }: { worktree: Worktree; base: string }) {
  const { data: pr, error, isLoading } = usePullRequest(worktree.path, !!worktree.branch);
  const { data: status } = useWorktreeStatus(worktree.path, 30_000);
  const { data: commits = [] } = useBranchLog(worktree.path, base);
  const [creating, setCreating] = useState(false);
  const [busy, setBusy] = useState(false);
  const refresh = useRefreshGit(worktree.path);
  const addComment = useReview((s) => s.add);

  async function push() {
    setBusy(true);
    try {
      await api.pushBranch(worktree.path);
      await refresh();
    } catch (e) {
      toast(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  async function importComments(p: PullRequest) {
    setBusy(true);
    try {
      const comments = await api.prReviewComments(worktree.path, p.number);
      for (const c of comments) {
        addComment(worktree.path, {
          file: c.path,
          side: c.side,
          line: c.line ?? 0,
          code: "",
          text: c.author ? `${c.author}: ${c.body}` : c.body,
        });
      }
      toast(
        comments.length === 0
          ? "The pull request has no review comments."
          : `Added ${comments.length} review comment${comments.length === 1 ? "" : "s"}; send them to the agent below.`,
        "info",
      );
    } catch (e) {
      toast(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Section title="Pull request">
      {isLoading && <p className="text-xs text-faint">Checking GitHub…</p>}
      {error && <p className="text-xs text-removed">{errorMessage(error)}</p>}
      {!isLoading && !error && !pr && (
        <p className="text-xs text-faint">
          {commits.length === 0
            ? "None yet. Once the branch has commits, you can open one here."
            : `None yet. Creating one pushes ${worktree.branch} and opens it on GitHub.`}
        </p>
      )}
      {!isLoading && !error && !pr && (
        <Button
          className="self-start"
          onClick={() => setCreating(true)}
          disabled={commits.length === 0}
          title={commits.length === 0 ? "Commit something first" : undefined}
        >
          <GitPullRequest size={13} />
          Create pull request…
        </Button>
      )}
      {pr && (
        <>
          <button
            onClick={() => openUrl(pr.url).catch((e) => toast(`Couldn't open the pull request: ${errorMessage(e)}`))}
            className="flex items-start gap-2 text-left text-xs hover:text-accent"
            title="Open on GitHub"
          >
            <PrStateIcon pr={pr} />
            <span className="min-w-0 flex-1">
              <span className="text-muted">#{pr.number}</span> {pr.title}
              <ExternalLink size={10} className="ml-1 inline text-faint" />
            </span>
          </button>
          <Checks pr={pr} />
          <div className="flex flex-wrap gap-2">
            {pr.state === "OPEN" && (status?.ahead ?? 0) > 0 && (
              <Button onClick={push} disabled={busy}>
                <ArrowUpFromLine size={13} />
                Push {status!.ahead} {status!.ahead === 1 ? "commit" : "commits"}
              </Button>
            )}
            {pr.state === "OPEN" && (
              <Button onClick={() => importComments(pr)} disabled={busy} title="Add the PR's review comments to this worktree's review">
                <MessageSquare size={13} />
                Get review comments
              </Button>
            )}
          </div>
        </>
      )}
      {creating && (
        <CreatePrDialog
          worktree={worktree}
          base={base}
          defaultTitle={commits[commits.length - 1]?.subject ?? ""}
          defaultBody={[...commits].reverse().map((c) => `- ${c.subject}`).join("\n")}
          onClose={() => setCreating(false)}
        />
      )}
    </Section>
  );
}

function PrStateIcon({ pr }: { pr: PullRequest }) {
  if (pr.state === "MERGED") return <GitMerge size={14} className="mt-px shrink-0 text-[#8250df]" />;
  if (pr.state === "CLOSED") return <GitPullRequest size={14} className="mt-px shrink-0 text-removed" />;
  return <GitPullRequest size={14} className={clsx("mt-px shrink-0", pr.isDraft ? "text-faint" : "text-added")} />;
}

function Checks({ pr }: { pr: PullRequest }) {
  const total = pr.checksPassed + pr.checksFailed + pr.checksPending;
  if (total === 0) return null;
  return (
    <span className="flex items-center gap-3 text-[11px] text-muted">
      {pr.checksFailed > 0 && (
        <span className="flex items-center gap-1 text-removed">
          <CircleX size={12} /> {pr.checksFailed} failed
        </span>
      )}
      {pr.checksPending > 0 && (
        <span className="flex items-center gap-1 text-warn">
          <CircleDot size={12} /> {pr.checksPending} running
        </span>
      )}
      {pr.checksPassed > 0 && (
        <span className="flex items-center gap-1 text-added">
          <CircleCheck size={12} /> {pr.checksPassed} passed
        </span>
      )}
    </span>
  );
}

function CreatePrDialog({
  worktree,
  base,
  defaultTitle,
  defaultBody,
  onClose,
}: {
  worktree: Worktree;
  base: string;
  defaultTitle: string;
  defaultBody: string;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const saved = useUi((s) => s.prDrafts[worktree.path]);
  const setSaved = useUi((s) => s.setPrDraft);
  const [title, setTitle] = useState(saved?.title ?? defaultTitle);
  const [body, setBody] = useState(saved?.body ?? defaultBody);
  const [draft, setDraft] = useState(false);
  const [busy, setBusy] = useState(false);
  const target = base.replace(/^origin\//, "");
  const [error, setError] = useState<string | null>(null);
  const { writerProfileId } = useAgentTarget(worktree.path);
  const [writing, setWriting] = useState(false);

  async function write() {
    setWriting(true);
    setError(null);
    try {
      const draft = await api.draftPullRequest(
        worktree.path,
        base,
        askPrompt("pullRequest", { base: target }),
        writerProfileId,
      );
      setTitle(draft.title);
      setBody(draft.body);
      setSaved(worktree.path, draft);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setWriting(false);
    }
  }

  async function create() {
    setBusy(true);
    setError(null);
    try {
      const pr = await api.createPullRequest(worktree.path, target, title.trim(), body, draft);
      qc.setQueryData(keys.pullRequest(worktree.path), pr);
      setSaved(worktree.path, null);
      await qc.invalidateQueries({ queryKey: keys.status(worktree.path) });
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      title="Create pull request"
      onClose={onClose}
      width={560}
      footer={
        <>
          <Button
            className="mr-auto"
            onClick={write}
            disabled={writing || busy}
            title="Let the agent write the title and description from the branch's commits and diff"
          >
            {writing ? <LoaderCircle size={13} className="animate-spin" /> : <Bot size={13} />}
            {writing ? "Writing…" : "Ask agent"}
          </Button>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={create} disabled={busy || writing || !title.trim()}>
            {busy && <LoaderCircle size={13} className="animate-spin" />}
            {busy ? "Pushing…" : "Push and create"}
          </Button>
        </>
      }
    >
      <p className="text-xs text-muted">
        Pushes <span className="font-mono text-fg">{worktree.branch}</span> to origin and opens a pull request into{" "}
        <span className="font-mono text-fg">{target}</span> with the GitHub CLI.
      </p>
      <Field label="Title">
        <Input
          value={title}
          onChange={(e) => {
            setTitle(e.target.value);
            if (saved) setSaved(worktree.path, { title: e.target.value, body });
          }}
          disabled={writing}
          autoFocus
        />
      </Field>
      <Field label="Description">
        <Textarea
          rows={8}
          value={body}
          onChange={(e) => {
            setBody(e.target.value);
            if (saved) setSaved(worktree.path, { title, body: e.target.value });
          }}
          disabled={writing}
          className="font-mono text-xs"
        />
      </Field>
      <Checkbox checked={draft} onChange={setDraft}>
        <span className="text-[13px]">Open as draft</span>
      </Checkbox>
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

/** Compact branch status for the worktree header; opens the branch view. */
export function BranchChip({ worktree, base, onOpen }: { worktree: Worktree; base: string; onOpen: () => void }) {
  const { data: commits = [] } = useBranchLog(worktree.path, base);
  const showPullRequests = useUi((s) => s.showPullRequests);
  const { data: pr } = usePullRequest(worktree.path, showPullRequests && !!worktree.branch);
  if (commits.length === 0 && !pr) return null;
  return (
    <button
      onClick={onOpen}
      title="Show commits and pull request"
      className="flex h-7 items-center gap-1.5 rounded-md px-2 text-xs text-muted hover:bg-hover hover:text-fg"
    >
      {pr ? <PrStateIcon pr={pr} /> : <GitCommitHorizontal size={13} />}
      {pr ? `#${pr.number}` : `${commits.length} ahead of ${base}`}
      {pr && pr.checksFailed > 0 && <CircleX size={12} className="text-removed" />}
      {pr && pr.checksFailed === 0 && pr.checksPending > 0 && <CircleDot size={12} className="text-warn" />}
    </button>
  );
}
