import { useQueries } from "@tanstack/react-query";

import type { Project, PullRequest, Worktree } from "./api";
import { pullRequestQuery, useProjects, worktreesQuery } from "./queries";
import { useUi } from "./store";

export interface WorktreePr {
  project: Project;
  worktree: Worktree;
  pr: PullRequest;
}

export const checksFailed = (pr: PullRequest) => pr.state === "OPEN" && pr.checksFailed > 0;

export const checksKey = ({ worktree, pr }: WorktreePr) => `${worktree.path}#${pr.number}`;

/** The pull requests of every worktree in every project, collapsed ones included. */
export function usePullRequests(): WorktreePr[] {
  const { data: projects = [] } = useProjects();
  const enabled = useUi((s) => s.showPullRequests);
  const lists = useQueries({ queries: projects.map((p) => worktreesQuery(p.id)) });
  const branches = projects.flatMap((project, i) =>
    (lists[i]?.data ?? []).filter((wt) => !wt.isMain && wt.branch).map((worktree) => ({ project, worktree })),
  );
  const prs = useQueries({ queries: branches.map(({ worktree }) => pullRequestQuery(worktree.path, enabled)) });
  if (!enabled) return [];
  return branches.flatMap((b, i) => {
    const pr = prs[i]?.data;
    return pr ? [{ ...b, pr }] : [];
  });
}

/** Open pull requests with failed checks that weren't skipped under Needs you. */
export function useFailedChecks(): WorktreePr[] {
  const dismissed = useUi((s) => s.dismissedChecks);
  return usePullRequests().filter((p) => checksFailed(p.pr) && !dismissed.includes(checksKey(p)));
}
