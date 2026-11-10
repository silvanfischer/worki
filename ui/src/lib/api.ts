import { Channel, invoke } from "@tauri-apps/api/core";

export type Isolation =
  | { kind: "none" }
  | { kind: "macUser"; user: string }
  | { kind: "dockerSandbox"; image: string; template?: string | null }
  | { kind: "docker"; image: string }
  | { kind: "podman"; image: string }
  | ({ kind: "ssh" } & Remote);
/** Isolations for agents on this Mac. */
export type LocalIsolation = Exclude<Isolation, { kind: "ssh" }>;
export type ProjectIsolation = { kind: "inherit" } | LocalIsolation;

export interface Remote {
  host: string;
  /** Unset uses ~/.ssh/config or your local user name. */
  user: string | null;
  /** Unset uses ~/.ssh/config or 22. */
  port: number | null;
  keepAlive: boolean;
  /** The saved host the connection settings come from. */
  hostId?: string | null;
}

export type SshAddress = Pick<Remote, "host" | "user" | "port">;

export interface SshHost {
  id: string;
  name: string;
  host: string;
  user: string | null;
  port: number | null;
}

/** `user@host:port`, leaving out what isn't set (matches the backend's naming). */
export function remoteLabel(r: Pick<Remote, "host" | "user" | "port">): string {
  return `${r.user ? `${r.user}@` : ""}${r.host}${r.port ? `:${r.port}` : ""}`;
}

export interface IsolationSettings {
  default: Isolation;
  stopOnQuit: boolean;
}

export type Fix = "provision" | "prepare" | "grantAccess" | "buildImage";

export interface Check {
  id: string;
  label: string;
  ok: boolean;
  detail: string | null;
  fix: Fix | null;
}

export const DEFAULT_AGENT_USER = "_worki";
export const DEFAULT_SANDBOX_IMAGE = "claude";
export const SANDBOX_IMAGES = ["claude", "codex", "gemini", "opencode", "shell"];
/** Built locally by Set up for Docker and Podman. */
export const DEFAULT_CONTAINER_IMAGE = "worki-agent:1";

export interface OpenerApp {
  id: string;
  name: string;
  path: string;
}

export interface OpenerSettings {
  apps: OpenerApp[];
  defaultApp: string | null;
}

export interface AgentStatus {
  profileId: string;
  name: string;
  executable: string;
  envPath: string | null;
  hostPath: string | null;
  canInstall: boolean;
  canShare: boolean;
  hasConfig: boolean;
  auth: { secret: string; set: boolean } | null;
}

export function isolationLabel(i: Isolation): string {
  if (i.kind === "none") return "None";
  if (i.kind === "macUser") return `User Isolation · ${i.user}`;
  if (i.kind === "ssh") return `SSH · ${remoteLabel(i)}`;
  if (i.kind === "docker") return `Docker · ${i.image}`;
  if (i.kind === "podman") return `Podman · ${i.image}`;
  return `Docker Sandbox · ${i.template ?? i.image}`;
}

export interface Project {
  id: string;
  name: string;
  repoPath: string;
  worktreeRoot: string;
  defaultBase: string;
  isolation: ProjectIsolation;
  setupScript: string | null;
  remote: Remote | null;
}

/** Where a project's agents run: its own host for remote projects, else its isolation. */
export function projectIsolation(project: Project, global: Isolation): Isolation {
  if (project.remote) return { kind: "ssh", ...project.remote };
  return project.isolation.kind === "inherit" ? global : project.isolation;
}

export interface ProjectPatch {
  name?: string;
  worktreeRoot?: string;
  defaultBase?: string;
  isolation?: ProjectIsolation;
  setupScript?: string;
  remote?: Remote;
}

export interface Worktree {
  path: string;
  branch: string | null;
  head: string;
  isMain: boolean;
  locked: boolean;
  prunable: boolean;
}

export interface ChangedFile {
  path: string;
  code: string;
}

export interface BranchChanges {
  base: string;
  files: ChangedFile[];
  insertions: number;
  deletions: number;
  commits: number;
  behind: number;
}

export interface WorktreeStatus {
  changedFiles: ChangedFile[];
  insertions: number;
  deletions: number;
  ahead: number;
  behind: number;
  upstream: string | null;
}

export interface SetupScripts {
  project: string | null;
  repoFile: string | null;
}

export type LogLine =
  | { stream: "info"; line: string }
  | { stream: "stdout"; line: string }
  | { stream: "stderr"; line: string };

export interface CreatedWorktree {
  worktree: Worktree;
  setupError: string | null;
}

export interface CommitInfo {
  sha: string;
  subject: string;
  author: string;
  time: number;
}

export type BaseUpdate = { kind: "upToDate" } | { kind: "updated" } | { kind: "conflicts"; files: string[] };

export interface PullRequest {
  number: number;
  title: string;
  url: string;
  state: "OPEN" | "CLOSED" | "MERGED";
  isDraft: boolean;
  checksPassed: number;
  checksFailed: number;
  checksPending: number;
}

export interface PrComment {
  path: string;
  line: number | null;
  side: "new" | "old";
  author: string;
  body: string;
}

export interface AgentProfile {
  id: string;
  name: string;
  command: string;
  install: string | null;
}

/** The flag an agent takes to run without permission prompts, for agents that have one. */
export function skipPermissionsFlag(command: string): string | null {
  const exe = command.trim().split(/\s+/)[0];
  if (exe === "claude") return "--dangerously-skip-permissions";
  if (exe === "codex") return "--dangerously-bypass-approvals-and-sandbox";
  return null;
}

/** `command` with the agent's skip-permissions flag added or removed. */
export function withSkipPermissions(command: string, on: boolean): string {
  const flag = skipPermissionsFlag(command);
  if (!flag) return command;
  const words = command.trim().split(/\s+/).filter((w) => w !== flag);
  return (on ? [...words, flag] : words).join(" ");
}

export interface AgentSettings {
  profiles: AgentProfile[];
  defaultProfile: string | null;
  /** Secret names per isolation scope (see `scopeKey`). */
  secrets: Record<string, string[]>;
}

export function scopeKey(i: Isolation): string {
  if (i.kind === "none") return "none";
  if (i.kind === "macUser") return `macUser:${i.user}`;
  if (i.kind === "ssh") return `ssh:${remoteLabel(i)}`;
  if (i.kind === "docker" || i.kind === "podman") return i.kind;
  return "dockerSandbox";
}

export type Activity = "working" | "waiting" | "idle";

export type SessionState = { kind: "running" } | { kind: "exited"; code: number };

export interface ModelUsage {
  model: string;
  input: number;
  output: number;
  /** All cache writes, including the 1-hour ones. */
  cacheWrite: number;
  cacheWrite1h: number;
  cacheRead: number;
}

export interface SessionInfo {
  id: string;
  worktreePath: string;
  profileId: string | null;
  title: string;
  runAs: string | null;
  /** Keeps running on its host when the connection ends; can be reconnected. */
  resumable: boolean;
  state: SessionState;
  activity: Activity;
  /** Text of the agent's last notification while it waits. */
  attention: string | null;
  createdAt: number;
  /** Tokens the agent's current conversation used, per model (Claude Code only). */
  usage: ModelUsage[];  /** Model, effort, context and the like, as the agent last reported them (Claude Code only). */
  agent: AgentState;
}

export interface AgentState {
  model: string | null;
  effort: string | null;
  fastMode: boolean | null;
  thinking: boolean | null;
  permissionMode: string | null;
  /** Share of the context window in use, in percent. */
  contextUsed: number | null;
  contextSize: number | null;
  costUsd: number | null;
  outputStyle: string | null;
  version: string | null;
}

export const api = {
  listProjects: () => invoke<Project[]>("list_projects"),
  addProject: (path: string) => invoke<Project>("add_project", { path }),
  addRemoteProject: (remote: Remote, path: string, password: string | null) =>
    invoke<Project>("add_remote_project", { remote, path, password }),
  sshHosts: () => invoke<SshHost[]>("ssh_hosts"),
  saveSshHost: (host: SshHost, password: string | null) => invoke<SshHost>("save_ssh_host", { host, password }),
  deleteSshHost: (id: string) => invoke<void>("delete_ssh_host", { id }),
  sshPasswordSet: ({ host, user, port }: SshAddress) => invoke<boolean>("ssh_password_set", { host, user, port }),
  setSshPassword: ({ host, user, port }: SshAddress, password: string | null) =>
    invoke<void>("set_ssh_password", { host, user, port, password }),
  updateProject: (id: string, patch: ProjectPatch) => invoke<Project>("update_project", { id, patch }),
  /** Resolves to a warning when the isolation cleanup had to be skipped. */
  removeProject: (id: string) => invoke<string | null>("remove_project", { id }),
  setupScripts: (id: string) => invoke<SetupScripts>("setup_scripts", { id }),
  listWorktrees: (projectId: string) => invoke<Worktree[]>("list_worktrees", { projectId }),
  listBranches: (projectId: string) => invoke<string[]>("list_branches", { projectId }),
  worktreeStatus: (path: string) => invoke<WorktreeStatus>("worktree_status", { path }),
  fileDiff: (worktreePath: string, file: string, base: string | null) =>
    invoke<string>("file_diff", { worktreePath, file, base }),
  commitChanges: (worktreePath: string, files: string[], message: string) =>
    invoke<string>("commit_changes", { worktreePath, files, message }),
  branchLog: (worktreePath: string, base: string | null) => invoke<CommitInfo[]>("branch_log", { worktreePath, base }),
  updateFromBase: (worktreePath: string, base: string) => invoke<BaseUpdate>("update_from_base", { worktreePath, base }),
  pullRequest: (worktreePath: string) => invoke<PullRequest | null>("pull_request", { worktreePath }),
  createPullRequest: (worktreePath: string, base: string, title: string, body: string, draft: boolean) =>
    invoke<PullRequest>("create_pull_request", { worktreePath, base, title, body, draft }),
  draftPullRequest: (worktreePath: string, base: string, instructions: string, profileId: string | null) =>
    invoke<{ title: string; body: string }>("draft_pull_request", { worktreePath, base, instructions, profileId }),
  compareBase: (worktreePath: string, base: string) => invoke<string>("compare_base", { worktreePath, base }),
  fetchRemote: (worktreePath: string) => invoke<void>("fetch_remote", { worktreePath }),
  pushBranch: (worktreePath: string) => invoke<void>("push_branch", { worktreePath }),
  prReviewComments: (worktreePath: string, number: number) =>
    invoke<PrComment[]>("pr_review_comments", { worktreePath, number }),
  environmentTemplates: (isolation: Isolation) => invoke<string[]>("environment_templates", { isolation }),
  saveEnvironmentTemplate: (projectId: string, worktreePath: string, tag: string, useForProject: boolean) =>
    invoke<Project>("save_environment_template", { projectId, worktreePath, tag, useForProject }),
  branchChanges: (worktreePath: string, base: string) =>
    invoke<BranchChanges>("branch_changes", { worktreePath, base }),
  createWorktree: (
    projectId: string,
    req: { name: string; base: string; runSetup: boolean; isolation: ProjectIsolation },
    onLog: (line: LogLine) => void,
  ) => {
    const channel = new Channel<LogLine>();
    channel.onmessage = onLog;
    return invoke<CreatedWorktree>("create_worktree", { projectId, req, onLog: channel });
  },
  removeWorktree: (projectId: string, req: { path: string; force: boolean; deleteBranch: boolean }) =>
    invoke<void>("remove_worktree", { projectId, req }),
  agentSettings: () => invoke<AgentSettings>("agent_settings"),
  saveAgentSettings: (settings: AgentSettings) => invoke<void>("save_agent_settings", { settings }),

  listSessions: () => invoke<SessionInfo[]>("list_sessions"),
  spawnSession: (
    projectId: string,
    worktreePath: string,
    profileId: string | null,
    cols: number,
    rows: number,
    resumeId: string | null = null,
    prompt: string | null = null,
  ) => invoke<SessionInfo>("spawn_session", { projectId, worktreePath, profileId, resumeId, prompt, cols, rows }),
  attachSession: (id: string, onData: (bytes: Uint8Array) => void) => {
    const channel = new Channel<ArrayBuffer | number[]>();
    channel.onmessage = (data) => onData(new Uint8Array(data));
    return invoke<number>("attach_session", { id, onData: channel });
  },
  detachSession: (id: string, sinkId: number) => invoke<void>("detach_session", { id, sinkId }),
  writeSession: (id: string, data: string) => invoke<void>("write_session", { id, data }),
  resizeSession: (id: string, cols: number, rows: number) => invoke<void>("resize_session", { id, cols, rows }),
  closeSession: (id: string) => invoke<void>("close_session", { id }),
  reconnectSession: (id: string, cols: number, rows: number) =>
    invoke<SessionInfo>("reconnect_session", { id, cols, rows }),
  restoreRemoteSessions: () => invoke<number>("restore_remote_sessions"),
  dismissAttention: (id: string) => invoke<void>("dismiss_attention", { id }),
  restoreAgentSessions: () => invoke<{ started: number; failed: string[] }>("restore_agent_sessions"),

  isolationSettings: () => invoke<IsolationSettings>("isolation_settings"),
  saveIsolationSettings: (settings: IsolationSettings) => invoke<void>("save_isolation_settings", { settings }),
  isolationDoctor: (isolation: Isolation, projectId: string | null) =>
    invoke<Check[]>("isolation_doctor", { isolation, projectId }),
  isolationFix: (isolation: Isolation, fix: Fix, projectId: string | null) =>
    invoke<void>("isolation_fix", { isolation, fix, projectId }),
  openerSettings: () => invoke<OpenerSettings>("opener_settings"),
  saveOpenerSettings: (settings: OpenerSettings) => invoke<void>("save_opener_settings", { settings }),
  openerAppFromPath: (path: string) => invoke<OpenerApp>("opener_app_from_path", { path }),
  openInApp: (app: string, target: string) => invoke<void>("open_in_app", { app, target }),
  appIcon: (app: string) => invoke<string | null>("app_icon", { app }),
  removeIsolation: (isolation: Isolation) => invoke<void>("isolation_remove", { isolation }),
  agentStatuses: (isolation: Isolation) => invoke<AgentStatus[]>("agent_statuses", { isolation }),
  shareAgent: (isolation: Isolation, executable: string) => invoke<string>("share_agent", { isolation, executable }),
  copyAgentConfig: (isolation: Isolation, executable: string) =>
    invoke<{ copied: string[]; skipped: string[] }>("copy_agent_config", { isolation, executable }),
  signInAgent: (isolation: Isolation, executable: string, onLog: (line: LogLine) => void) => {
    const channel = new Channel<LogLine>();
    channel.onmessage = onLog;
    return invoke<void>("sign_in_agent", { isolation, executable, onLog: channel });
  },
  worktreeIsolation: (projectId: string, worktreePath: string) =>
    invoke<{ choice: ProjectIsolation; effective: Isolation }>("worktree_isolation", { projectId, worktreePath }),
  setWorktreeIsolation: (projectId: string, worktreePath: string, choice: ProjectIsolation) =>
    invoke<Isolation>("set_worktree_isolation", { projectId, worktreePath, choice }),
  buildIsolationImage: (isolation: Isolation, onLog: (line: LogLine) => void) => {
    const channel = new Channel<LogLine>();
    channel.onmessage = onLog;
    return invoke<void>("isolation_build_image", { isolation, onLog: channel });
  },
  installAgent: (isolation: Isolation, command: string, onLog: (line: LogLine) => void) => {
    const channel = new Channel<LogLine>();
    channel.onmessage = onLog;
    return invoke<void>("install_agent", { isolation, command, onLog: channel });
  },
  secretStatus: (scope: string, names: string[]) => invoke<[string, boolean][]>("secret_status", { scope, names }),
  setSecret: (scope: string, name: string, value: string) => invoke<void>("set_secret", { scope, name, value }),
  deleteSecret: (scope: string, name: string) => invoke<void>("delete_secret", { scope, name }),
};

export function worktreeLabel(wt: Worktree): string {
  return wt.branch ?? `detached @ ${wt.head.slice(0, 7)}`;
}

export function errorMessage(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
