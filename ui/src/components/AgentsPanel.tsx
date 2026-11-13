import { useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import { CircleCheck, CircleX, KeyRound, LoaderCircle, RefreshCw, Sparkles } from "lucide-react";
import { useState } from "react";

import {
  type AgentStatus,
  api,
  errorMessage,
  type Isolation,
  type LogLine,
  scopeKey,
} from "../lib/api";
import { keys, useAgentStatuses, useRefreshSecrets } from "../lib/queries";
import { Button, ErrorText, IconButton, LogView } from "./ui";

type Action = "share" | "config" | "token" | "removeToken";

/** Per-agent setup inside an isolated environment: reuse your installation, copy config, sign in. */
export function AgentsPanel({ isolation }: { isolation: Isolation }) {
  const qc = useQueryClient();
  const { data: agents, isFetching, error, refetch } = useAgentStatuses(isolation);
  const [running, setRunning] = useState<{ exe: string; action: Action } | null>(null);
  const [result, setResult] = useState<{ exe: string; lines: LogLine[]; error?: string } | null>(null);
  const refreshSecrets = useRefreshSecrets();
  const sandbox = isolation.kind === "dockerSandbox" || isolation.kind === "docker" || isolation.kind === "podman";
  const envNoun = isolation.kind === "dockerSandbox" ? "sandbox" : "container";
  const envNouns = isolation.kind === "dockerSandbox" ? "sandboxes" : "containers";
  const user = isolation.kind === "macUser" ? isolation.user : isolation.kind === "ssh" ? isolation.host : "the sandbox";
  const where = isolation.kind === "ssh" ? `on ${isolation.host}` : `for ${user}`;

  async function run(agent: AgentStatus, action: Action) {
    const exe = agent.executable;
    setRunning({ exe, action });
    const lines: LogLine[] = [];
    const push = (line: LogLine) => {
      lines.push(line);
      setResult({ exe, lines: [...lines] });
    };
    setResult({ exe, lines: [] });
    try {
      if (action === "share") push({ stream: "info", line: `Linked ${await api.shareAgent(isolation, exe)}` });
      if (action === "config") {
        const { copied, skipped } = await api.copyAgentConfig(isolation, exe);
        push({ stream: "info", line: copied.length ? `Copied: ${copied.join(", ")}` : "Nothing to copy." });
        if (skipped.length)
          push({ stream: "info", line: `Skipped: ${skipped.join("; ")}` });
      }
      if (action === "token" && agent.auth) {
        await api.signInAgent(isolation, exe, push);
        push({ stream: "info", line: `Saved ${agent.auth.secret} in your Keychain for agents running as ${user}.` });
        await refreshSecrets();
      }
      if (action === "removeToken" && agent.auth) {
        await api.deleteSecret(scopeKey(isolation), agent.auth.secret);
        push({ stream: "info", line: `Removed ${agent.auth.secret}.` });
        await refreshSecrets();
      }
    } catch (e) {
      setResult({ exe, lines: [...lines], error: errorMessage(e) });
    } finally {
      setRunning(null);
      qc.invalidateQueries({ queryKey: keys.agentStatuses(isolation) });
    }
  }

  if (isolation.kind === "none") return null;

  return (
    <div className="flex flex-col gap-1 rounded-lg border border-line bg-panel p-2">
      <div className="flex items-center justify-between px-1 pb-1">
        <span className="text-[11px] font-semibold tracking-wide text-faint uppercase">
          {sandbox ? `Agents in ${envNouns}` : `Agents ${where}`}
        </span>
        <IconButton label="Refresh" onClick={() => refetch()} disabled={isFetching}>
          <RefreshCw size={12} className={clsx(isFetching && "animate-spin")} />
        </IconButton>
      </div>
      {!agents && isFetching && <p className="px-1 py-1 text-xs text-faint">Looking for agents…</p>}
      {error && <ErrorText>{String(error)}</ErrorText>}
      {agents?.map((a) => {
        const busy = running?.exe === a.executable ? running.action : null;
        const btn = (action: Action, label: string, show: boolean, primary = false) =>
          show && (
            <Button
              key={action}
              variant={primary ? "primary" : "secondary"}
              onClick={() => run(a, action)}
              disabled={running !== null}
            >
              {busy === action && <LoaderCircle size={12} className="animate-spin" />}
              {label}
            </Button>
          );
        return (
          <div key={a.profileId} className="flex flex-col gap-1.5 rounded-md px-1 py-1.5">
            <div className="flex items-start gap-2">
              {a.envPath ? (
                <CircleCheck size={14} className="mt-0.5 shrink-0 text-added" />
              ) : (
                <CircleX size={14} className="mt-0.5 shrink-0 text-removed" />
              )}
              <div className="min-w-0 flex-1">
                <div className="text-[13px]">
                  {a.name}{" "}
                  <span className="text-faint">
                    {a.envPath ? `is installed ${where}` : `is not installed ${where}`}
                  </span>
                </div>
                <div className="selectable truncate font-mono text-[11px] text-faint">
                  {a.envPath ?? (a.hostPath ? `Yours: ${a.hostPath}` : `Not installed for you either`)}
                </div>
                {sandbox && (
                  <div className="mt-0.5 flex items-center gap-1 text-[11px] text-faint">
                    <Sparkles size={11} className={a.inBaseImage ? "text-added" : ""} />
                    {a.inBaseImage
                      ? "Part of the base image"
                      : `Installed in a worktree's ${envNoun} the first time you start it there`}
                  </div>
                )}
                {a.auth && (
                  <div className="mt-0.5 flex items-center gap-1 text-[11px] text-faint">
                    <KeyRound size={11} className={a.auth.set ? "text-added" : ""} />
                    {a.auth.set ? `Signed in with a token (${a.auth.secret})` : "Not signed in"}
                  </div>
                )}
              </div>
            </div>
            <div className="ml-6 flex flex-wrap gap-1.5">
              {btn("share", "Use my installation", a.canShare && !a.envPath && !!a.hostPath, true)}
              {btn("config", "Copy my config", a.hasConfig)}
              {btn("token", a.auth?.set ? "Sign in again" : "Sign in", !!a.auth)}
              {btn("removeToken", "Remove token", !!a.auth?.set)}
            </div>
            {result?.exe === a.executable && (result.lines.length > 0 || result.error) && (
              <ResultLog lines={result.lines} error={result.error} />
            )}
          </div>
        );
      })}
      {sandbox && (
        <p className="px-1 pt-1 text-xs leading-relaxed text-faint">
          An agent is installed in a worktree's {envNoun} the first time you start it there, or a new {envNoun} gets it
          from a snapshot of one that has it. Copy my config applies to every {envNoun}, new ones included.
        </p>
      )}
      <p className="px-1 pt-1 text-xs leading-relaxed text-faint">
        Sign in runs <span className="font-mono">claude setup-token</span> as you and approves in your browser. The
        token uses your subscription and is passed to {user}'s sessions; your own login stays untouched. Codex
        signs in per environment with a code you approve in the browser the first time it starts there.
      </p>
    </div>
  );
}

function ResultLog({ lines, error }: { lines: LogLine[]; error?: string }) {
  return (
    <div className="ml-6 flex flex-col gap-1">
      {lines.length > 0 && <LogView lines={lines} className="max-h-40 bg-elevated" />}
      <ErrorText>{error}</ErrorText>
    </div>
  );
}
