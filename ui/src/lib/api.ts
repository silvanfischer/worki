import { Channel, invoke } from "@tauri-apps/api/core";

export type Isolation =
  | { kind: "none" }
  | { kind: "macUser"; user: string }
  | { kind: "dockerSandbox"; image: string }
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
}

/** `user@host:port`, leaving out what isn't set (matches the backend's naming). */
export function remoteLabel(r: Pick<Remote, "host" | "user" | "port">): string {
  return `${r.user ? `${r.user}@` : ""}${r.host}${r.port ? `:${r.port}` : ""}`;
}

export interface IsolationSettings {
  default: Isolation;
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
  return `Docker Sandbox · ${i.image}`;
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

export interface AgentProfile {
  id: string;
  name: string;
  command: string;
  install: string | null;
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
}

export const api = {
  listProjects: () => invoke<Project[]>("list_projects"),
  addProject: (path: string) => invoke<Project>("add_project", { path }),
  addRemoteProject: (remote: Remote, path: string, password: string | null) =>
    invoke<Project>("add_remote_project", { remote, path, password }),
  sshPasswordSet: (remote: Remote) => invoke<boolean>("ssh_password_set", { remote }),
  setSshPassword: (remote: Remote, password: string | null) => invoke<void>("set_ssh_password", { remote, password }),
  updateProject: (id: string, patch: ProjectPatch) => invoke<Project>("update_project", { id, patch }),
  /** Resolves to a warning when the isolation cleanup had to be skipped. */
  removeProject: (id: string) => invoke<string | null>("remove_project", { id }),
  setupScripts: (id: string) => invoke<SetupScripts>("setup_scripts", { id }),
  listWorktrees: (projectId: string) => invoke<Worktree[]>("list_worktrees", { projectId }),
  listBranches: (projectId: string) => invoke<string[]>("list_branches", { projectId }),
  worktreeStatus: (path: string) => invoke<WorktreeStatus>("worktree_status", { path }),
  fileDiff: (worktreePath: string, file: string) => invoke<string>("file_diff", { worktreePath, file }),
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
  spawnSession: (projectId: string, worktreePath: string, profileId: string | null, cols: number, rows: number) =>
    invoke<SessionInfo>("spawn_session", { projectId, worktreePath, profileId, cols, rows }),
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
