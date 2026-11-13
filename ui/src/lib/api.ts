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
  canShare: boolean;
  hasConfig: boolean;
  inBaseImage: boolean;
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
  /** What the branch was started from, when worki created it. */
  base: string | null;
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

export interface FailedCheck {
  name: string;
  url: string;
  description: string;
  /** The end of the failed GitHub Actions job's log, around its error. */
  log: string | null;
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

/** Agents worki can run as a chat instead of in a terminal. */
export function canChat(profile: AgentProfile): boolean {
  return ["claude", "codex"].includes(profile.command.trim().split(/\s+/)[0]);
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

/** A commit an agent proposes: its message and which of the files to commit. */
export interface CommitDraft {
  message: string;
  files: string[];
}

export type SnapshotEngine = "dockerSandbox" | "docker" | "podman";

/** A saved environment snapshot, with the settings that start new environments from it. */
export interface Snapshot {
  engine: SnapshotEngine;
  name: string;
  /** Unix seconds, when the engine says. */
  created: number | null;
  usedBy: string[];
}

/** How worki runs an agent once in the background to write a text. */
export interface Writer {
  /** The Claude session whose conversation `{fork}` copies. */
  fork: string | null;
  /** Command with `{fork}` and `{prompt}` placeholders; null uses `DEFAULT_WRITER_COMMAND`. */
  command: string | null;
}

export const DEFAULT_WRITER_COMMAND = "claude --model haiku {fork} --no-session-persistence -p {prompt}";

export interface AgentSettings {
  profiles: AgentProfile[];
  defaultProfile: string | null;
  /** Secret names per isolation scope (see `scopeKey`). */
  secrets: Record<string, string[]>;
  remoteControl: boolean;
  /** Agent configs copied into every new environment, per isolation scope. */
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

export interface ChatQuestion {
  question: string;
  header: string | null;
  options: { label: string; description: string | null }[];
  multiSelect: boolean;
}

export interface BackgroundTask {
  id: string;
  /** Such as `local_bash` or `local_agent`. */
  kind: string;
  description: string;
}

/** `off`, `connecting`, or the agent's own connection state; others are shown as they come. */
export type RemoteControlState = "off" | "connecting" | "ready" | "connected" | "reconnecting" | (string & {});

/** Status words the agent reports between tool calls; others are shown as they come. */
export type AgentStatusKind = "requesting" | "thinking" | "writing" | "tool" | "compacting" | (string & {});

/** What happened in a chat session, the same for every agent. `parent` names the tool call a
 * subagent works for. */
export type ChatEvent = (
  | { type: "session"; model: string | null; permissionMode: string | null; effort: string | null }
  | { type: "options"; models: ModelChoice[]; permissionModes: string[] }
  | { type: "user"; id: string | null; text: string; attachments: string[] }
  | { type: "userRead"; id: string }
  | { type: "textDelta"; id: string; text: string; parent: string | null }
  | { type: "text"; id: string; text: string; parent: string | null }
  | { type: "toolCall"; id: string; name: string; input: unknown; parent: string | null }
  | { type: "toolResult"; id: string; output: string; isError: boolean; parent: string | null }
  | { type: "permission"; requestId: string; tool: string; input: unknown; questions: ChatQuestion[] }
  | { type: "permissionResolved"; requestId: string; allowed: boolean; elsewhere: boolean }
  | { type: "notice"; text: string }
  | { type: "status"; status: AgentStatusKind; tool: string | null }
  | { type: "backgroundTasks"; tasks: BackgroundTask[] }
  | { type: "suggestion"; text: string }
  | { type: "turnEnded"; ok: boolean; interrupted: boolean; error: string | null }
  | { type: "exited"; code: number; error: string | null }
) & {
  /** When worki recorded the event, in Unix milliseconds. */
  at?: number;
};

export interface ModelChoice {
  /** What to switch to, such as an alias like `opus`. */
  value: string;
  /** The model it currently means. */
  model: string | null;
  label: string;
  description: string | null;
  effortLevels: string[];
}

/** A file worki reads and sends along, or an image pasted from the clipboard (base64). */
export type ChatAttachment =
  | { kind: "file"; path: string }
  | { kind: "image"; name: string; mediaType: string; data: string };

export type ChatSetting =
  | { kind: "model"; value: string }
  | { kind: "permissionMode"; value: string }
  | { kind: "effort"; value: string }
  | { kind: "remoteControl"; value: boolean };

export type PermissionAnswer =
  | { kind: "allow" }
  | { kind: "deny"; message: string | null }
  | { kind: "answers"; answers: Record<string, string> };

/** How worki talks to a session: through a terminal, or the agent's JSON protocol. */
export type SessionKind = "terminal" | "chat";

export interface SessionInfo {
  id: string;
  kind: SessionKind;
  worktreePath: string;
  profileId: string | null;
  title: string;
  runAs: string | null;
  /** Keeps running on its host when the connection ends; can be reconnected. */
  resumable: boolean;
  /** The worktree's shell in the terminal panel, not a session tab. */
  panel: boolean;
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
  /** Only reported by chat agents that can be continued on claude.ai/code. */
  remoteControl: { state: RemoteControlState; url: string | null } | null;
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
  draftPullRequest: (worktreePath: string, base: string, instructions: string, writer: Writer) =>
    invoke<{ title: string; body: string }>("draft_pull_request", { worktreePath, base, instructions, writer }),
  draftCommits: (worktreePath: string, files: string[], instructions: string, writer: Writer) =>
    invoke<CommitDraft[]>("draft_commits", { worktreePath, files, instructions, writer }),
  compareBase: (worktreePath: string, base: string) => invoke<string>("compare_base", { worktreePath, base }),
  fetchRemote: (worktreePath: string) => invoke<void>("fetch_remote", { worktreePath }),
  pushBranch: (worktreePath: string) => invoke<void>("push_branch", { worktreePath }),
  prReviewComments: (worktreePath: string, number: number) =>
    invoke<PrComment[]>("pr_review_comments", { worktreePath, number }),
  githubImage: (worktreePath: string, url: string) => invoke<string>("github_image", { worktreePath, url }),
  prFailedChecks: (worktreePath: string, number: number) =>
    invoke<FailedCheck[]>("pr_failed_checks", { worktreePath, number }),
  environmentTemplates: (isolation: Isolation) => invoke<string[]>("environment_templates", { isolation }),
  snapshots: () => invoke<Snapshot[]>("snapshots"),
  writeClipboard: (text: string) => invoke<void>("write_clipboard", { text }),
  saveFile: (path: string, contents: Uint8Array) => invoke<void>("save_file", { path, contents: Array.from(contents) }),
  deleteSnapshot: (engine: SnapshotEngine, name: string) => invoke<void>("delete_snapshot", { engine, name }),
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
  saveAgentProfiles: (profiles: AgentProfile[], defaultProfile: string | null) =>
    invoke<void>("save_agent_profiles", { profiles, defaultProfile }),
  setRemoteControl: (enabled: boolean) => invoke<void>("set_remote_control", { enabled }),

  listSessions: () => invoke<SessionInfo[]>("list_sessions"),
  /** Starts an agent as a chat; `attachSession` then streams one `ChatEvent` as JSON per line. */
  spawnChatSession: (projectId: string, worktreePath: string, profileId: string, resumeId: string | null = null) =>
    invoke<SessionInfo>("spawn_chat_session", { projectId, worktreePath, profileId, resumeId }),
  sendChatMessage: (id: string, text: string, attachments: ChatAttachment[] = []) =>
    invoke<void>("send_chat_message", { id, text, attachments }),
  answerChatPermission: (id: string, requestId: string, answer: PermissionAnswer) =>
    invoke<void>("answer_chat_permission", { id, requestId, answer }),
  interruptChat: (id: string) => invoke<void>("interrupt_chat", { id }),
  /** Moves the agent's foreground commands to the background, so it reads queued messages now. */
  backgroundChatCommands: (id: string) => invoke<void>("background_chat_commands", { id }),
  stopChatTasks: (id: string, taskIds: string[]) => invoke<void>("stop_chat_tasks", { id, taskIds }),
  /** The command that continues the session in the user's own terminal, where it ran. */
  sessionContinueCommand: (id: string) => invoke<string>("session_continue_command", { id }),
  changeChatSetting: (id: string, setting: ChatSetting) => invoke<void>("change_chat_setting", { id, setting }),
  spawnSession: (
    projectId: string,
    worktreePath: string,
    profileId: string | null,
    cols: number,
    rows: number,
    resumeId: string | null = null,
    prompt: string | null = null,
  ) => invoke<SessionInfo>("spawn_session", { projectId, worktreePath, profileId, resumeId, prompt, cols, rows }),
  /** The user's own shell in the worktree, outside its isolation, for the terminal panel. */
  spawnWorktreeShell: (projectId: string, worktreePath: string, cols: number, rows: number) =>
    invoke<SessionInfo>("spawn_worktree_shell", { projectId, worktreePath, cols, rows }),
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
