import { useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import { CircleCheck, CircleX, KeyRound, LoaderCircle, RefreshCw } from "lucide-react";
import { useState } from "react";

import { type AgentStatus, api, errorMessage, type Isolation, type LogLine, scopeKey } from "../lib/api";
import { keys, useAgentStatuses } from "../lib/queries";
import { Button, ErrorText, IconButton, Input, LogView } from "./ui";

type Action = "share" | "install" | "config" | "token" | "removeToken";

/** Per-agent setup inside an isolated environment: reuse or install it, copy config, sign in. */
export function AgentsPanel({ isolation }: { isolation: Isolation }) {
  const qc = useQueryClient();
  const { data: agents, isFetching, error, refetch } = useAgentStatuses(isolation);
  const [running, setRunning] = useState<{ exe: string; action: Action } | null>(null);
  const [result, setResult] = useState<{ exe: string; lines: LogLine[]; error?: string } | null>(null);
  const [keyFor, setKeyFor] = useState<string | null>(null);
  const [key, setKey] = useState("");
  const sandbox = isolation.kind === "dockerSandbox" || isolation.kind === "docker" || isolation.kind === "podman";
  const envNoun = isolation.kind === "dockerSandbox" ? "sandbox" : "container";
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
      if (action === "install") await api.installAgent(isolation, exe, push);
      if (action === "config") {
        const { copied, skipped } = await api.copyAgentConfig(isolation, exe);
        push({ stream: "info", line: copied.length ? `Copied: ${copied.join(", ")}` : "Nothing to copy." });
        if (skipped.length)
          push({ stream: "info", line: `Skipped ${skipped.join(", ")}: they run scripts from your home folder.` });
      }
      if (action === "token" && agent.auth) {
        if (exe === "claude") await api.signInAgent(isolation, exe, push);
        else {
          await api.setSecret(scopeKey(isolation), agent.auth.secret, key.trim());
          setKeyFor(null);
          setKey("");
        }
        push({ stream: "info", line: `Saved ${agent.auth.secret} in your Keychain for agents running as ${user}.` });
        qc.invalidateQueries({ queryKey: keys.agentSettings });
      }
      if (action === "removeToken" && agent.auth) {
        await api.deleteSecret(scopeKey(isolation), agent.auth.secret);
        push({ stream: "info", line: `Removed ${agent.auth.secret}.` });
        qc.invalidateQueries({ queryKey: keys.agentSettings });
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
          {sandbox ? `Agents in ${envNoun}s` : `Agents ${where}`}
        </span>
        <IconButton label="Refresh" onClick={() => refetch()} disabled={isFetching}>
          <RefreshCw size={12} className={clsx(isFetching && "animate-spin")} />
        </IconButton>
      </div>
      {!agents && isFetching && <p className="px-1 py-1 text-xs text-faint">Looking for agents…</p>}
      {error && <ErrorText>{String(error)}</ErrorText>}
      {agents?.map((a) => {
        const busy = running?.exe === a.executable ? running.action : null;
        const asksKey = a.executable !== "claude";
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
              {btn("install", a.envPath ? "Reinstall" : "Install", a.canInstall, !a.envPath && !(a.canShare && a.hostPath))}
              {btn("config", "Copy my config", a.hasConfig)}
              {asksKey ? (
                !!a.auth &&
                keyFor !== a.executable && (
                  <Button onClick={() => setKeyFor(a.executable)} disabled={running !== null}>
                    {a.auth.set ? "Change API key" : "Add API key"}
                  </Button>
                )
              ) : (
                btn("token", a.auth?.set ? "Sign in again" : "Sign in", !!a.auth)
              )}
              {btn("removeToken", "Remove token", !!a.auth?.set)}
            </div>
            {asksKey && a.auth && keyFor === a.executable && (
              <form
                className="ml-6 flex items-center gap-1.5"
                onSubmit={(e) => {
                  e.preventDefault();
                  if (key.trim()) run(a, "token");
                }}
              >
                <Input
                  type="password"
                  autoFocus
                  value={key}
                  onChange={(e) => setKey(e.target.value)}
                  placeholder={`${a.auth.secret}, e.g. sk-…`}
                  className="min-w-0 flex-1 font-mono text-xs"
                />
                <Button type="submit" variant="primary" disabled={!key.trim() || running !== null}>
                  Save
                </Button>
                <Button type="button" onClick={() => (setKeyFor(null), setKey(""))}>
                  Cancel
                </Button>
              </form>
            )}
            {result?.exe === a.executable && (result.lines.length > 0 || result.error) && (
              <ResultLog lines={result.lines} error={result.error} />
            )}
          </div>
        );
      })}
      {sandbox && (
        <p className="px-1 pt-1 text-xs leading-relaxed text-faint">
          Install and Copy my config apply to every worki {envNoun} and are repeated automatically in each new one.
        </p>
      )}
      <p className="px-1 pt-1 text-xs leading-relaxed text-faint">
        Sign in runs <span className="font-mono">claude setup-token</span> as you and approves in your browser. The
        token uses your subscription and is passed to {user}'s sessions; your own login stays untouched. Codex
        (experimental) signs in with an OpenAI API key, kept in your Keychain.
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
