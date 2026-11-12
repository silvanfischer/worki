import { queryOptions, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";

import { api, type Isolation, type Project, type Remote, type SessionInfo } from "./api";
import { useUi } from "./store";

export const keys = {
  projects: ["projects"] as const,
  worktrees: (projectId: string) => ["worktrees", projectId] as const,
  status: (path: string) => ["status", path] as const,
  diff: (worktreePath: string, file: string, base: string | null) => ["diff", worktreePath, file, base] as const,
  branchChanges: (worktreePath: string, base: string) => ["branchChanges", worktreePath, base] as const,
  worktreeIsolation: (path: string) => ["worktreeIsolation", path] as const,
  branches: (projectId: string) => ["branches", projectId] as const,
  setupScripts: (projectId: string) => ["setupScripts", projectId] as const,
  sessions: ["sessions"] as const,
  agentSettings: ["agentSettings"] as const,
  isolationSettings: ["isolationSettings"] as const,
  allDoctor: ["doctor"] as const,
  doctor: (isolation: Isolation, projectId: string | null) => ["doctor", isolation, projectId] as const,
  allAgentStatuses: ["agentStatuses"] as const,
  agentStatuses: (isolation: Isolation) => ["agentStatuses", isolation] as const,
  allSecretStatus: ["secretStatus"] as const,
  secretStatus: (scope: string, names: string[]) => ["secretStatus", scope, names] as const,
  sshPasswordSet: (remote: Pick<Remote, "host" | "user" | "port"> | null) =>
    ["sshPassword", remote && { host: remote.host, user: remote.user, port: remote.port }] as const,
  sshHosts: ["sshHosts"] as const,
  branchLog: (path: string, base: string) => ["branchLog", path, base] as const,
  pullRequest: (path: string) => ["pullRequest", path] as const,
  environmentTemplates: (kind: string) => ["environmentTemplates", kind] as const,
  snapshots: ["environmentTemplates", "snapshots"] as const,
  openers: ["openers"] as const,
  appIcon: (app: string) => ["appIcon", app] as const,
};

export const useOpenerSettings = () => useQuery({ queryKey: keys.openers, queryFn: api.openerSettings });

export const useAppIcon = (app: string) =>
  useQuery({ queryKey: keys.appIcon(app), queryFn: () => api.appIcon(app), staleTime: Infinity });

/** `origin/<base>` when it exists, else `base`: what the worktree is compared with. */
export const useCompareBase = (worktreePath: string, base: string | null) =>
  useQuery({
    queryKey: ["compareBase", worktreePath, base],
    queryFn: () => api.compareBase(worktreePath, base!),
    enabled: !!base,
    staleTime: 60_000,
  }).data ?? base;

export const useBranchLog = (worktreePath: string, base: string | null) =>
  useQuery({
    queryKey: keys.branchLog(worktreePath, base ?? ""),
    queryFn: () => api.branchLog(worktreePath, base),
    refetchInterval: 10_000,
  });

export const pullRequestQuery = (worktreePath: string, enabled: boolean) =>
  queryOptions({
    queryKey: keys.pullRequest(worktreePath),
    queryFn: () => api.pullRequest(worktreePath),
    enabled,
    refetchInterval: 60_000,
    retry: false,
  });

export const usePullRequest = (worktreePath: string, enabled: boolean) => useQuery(pullRequestQuery(worktreePath, enabled));

export const useEnvironmentTemplates = (isolation: Isolation) =>
  useQuery({
    queryKey: keys.environmentTemplates(isolation.kind),
    queryFn: () => api.environmentTemplates(isolation),
    staleTime: 30_000,
  });

export const useSnapshots = () => useQuery({ queryKey: keys.snapshots, queryFn: api.snapshots });

export const useIsolationSettings = () =>
  useQuery({ queryKey: keys.isolationSettings, queryFn: api.isolationSettings });

export const useDoctor = (isolation: Isolation | null, projectId: string | null) =>
  useQuery({
    queryKey: keys.doctor(isolation ?? { kind: "none" }, projectId),
    queryFn: () => api.isolationDoctor(isolation!, projectId),
    enabled: !!isolation && isolation.kind !== "none",
    staleTime: 0,
  });

/** The isolation a project's worktrees inherit; undefined while it depends on unloaded global settings. */
export function useProjectDefaultIsolation(project: Project): Isolation | undefined {
  const { data: settings } = useIsolationSettings();
  return project.isolation.kind === "inherit" ? settings?.default : project.isolation;
}

/** Refreshes everything that shows which secrets are stored, after one was saved or removed. */
export function useRefreshSecrets() {
  const qc = useQueryClient();
  return async () => {
    await qc.invalidateQueries({ queryKey: keys.agentSettings });
    qc.invalidateQueries({ queryKey: keys.allSecretStatus });
    qc.invalidateQueries({ queryKey: keys.allAgentStatuses });
  };
}

export const useAgentStatuses = (isolation: Isolation) =>
  useQuery({ queryKey: keys.agentStatuses(isolation), queryFn: () => api.agentStatuses(isolation), staleTime: 0 });

export const useSshHosts = () => useQuery({ queryKey: keys.sshHosts, queryFn: api.sshHosts });

export const useSshPasswordSet = (remote: Pick<Remote, "host" | "user" | "port"> | null) =>
  useQuery({
    queryKey: keys.sshPasswordSet(remote),
    queryFn: () => api.sshPasswordSet(remote!),
    enabled: !!remote,
  });

export const useSecretStatus = (scope: string, names: string[]) =>
  useQuery({ queryKey: keys.secretStatus(scope, names), queryFn: () => api.secretStatus(scope, names) });

const tabSessions = (all: SessionInfo[]) => all.filter((s) => !s.panel);

/** Sessions shown as tabs; the shells of the terminal panels are left out. */
export const useSessions = () => useQuery({ queryKey: keys.sessions, queryFn: api.listSessions, select: tabSessions });

/** The shell in a worktree's terminal panel, once it was started. */
export const useWorktreeShell = (worktreePath: string) =>
  useQuery({
    queryKey: keys.sessions,
    queryFn: api.listSessions,
    select: (all) => all.find((s) => s.panel && s.worktreePath === worktreePath),
  });

export const useAgentSettings = () => useQuery({ queryKey: keys.agentSettings, queryFn: api.agentSettings });

export const useProjects = () => useQuery({ queryKey: keys.projects, queryFn: api.listProjects });

export const worktreesQuery = (projectId: string) =>
  queryOptions({
    queryKey: keys.worktrees(projectId),
    queryFn: () => api.listWorktrees(projectId),
    refetchInterval: 15_000,
  });

export const useWorktrees = (projectId: string) => useQuery(worktreesQuery(projectId));

export const useWorktreeStatus = (path: string | null, refetchInterval = 15_000) =>
  useQuery({
    queryKey: keys.status(path ?? ""),
    queryFn: () => api.worktreeStatus(path!),
    enabled: !!path,
    refetchInterval,
  });

export const useFileDiff = (worktreePath: string, file: string, base: string | null) =>
  useQuery({
    queryKey: keys.diff(worktreePath, file, base),
    queryFn: () => api.fileDiff(worktreePath, file, base),
    refetchInterval: 5_000,
  });

export const useBranchChanges = (worktreePath: string, base: string | null) =>
  useQuery({
    queryKey: keys.branchChanges(worktreePath, base ?? ""),
    queryFn: () => api.branchChanges(worktreePath, base!),
    enabled: !!base,
    refetchInterval: 5_000,
  });

export const useWorktreeIsolation = (projectId: string, worktreePath: string) =>
  useQuery({
    queryKey: keys.worktreeIsolation(worktreePath),
    queryFn: () => api.worktreeIsolation(projectId, worktreePath),
  });

export const useBranches = (projectId: string) =>
  useQuery({ queryKey: keys.branches(projectId), queryFn: () => api.listBranches(projectId) });

export const useSetupScripts = (projectId: string) =>
  useQuery({ queryKey: keys.setupScripts(projectId), queryFn: () => api.setupScripts(projectId) });

export function useAddProject() {
  const qc = useQueryClient();
  const select = useUi((s) => s.select);
  return useMutation({
    mutationFn: async () => {
      const path = await open({ directory: true, title: "Choose a git repository" });
      return path ? api.addProject(path) : null;
    },
    onSuccess: (project) => {
      if (!project) return;
      qc.invalidateQueries({ queryKey: keys.projects });
      select(project.id, project.repoPath);
    },
  });
}
