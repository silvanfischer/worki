import clsx from "clsx";
import { Check, Copy, Gauge } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { type ReactNode, useEffect, useRef, useState } from "react";

import { api, errorMessage, type SessionInfo } from "../lib/api";
import { toast } from "../lib/toast";
import { useCopy } from "../lib/useCopy";
import { formatTokens, totalUsage } from "../lib/usage";

export const PERMISSION_MODES: Record<string, string> = {
  default: "Ask before edits",
  acceptEdits: "Accept edits",
  plan: "Plan",
  auto: "Auto",
  dontAsk: "Don't ask",
  bypassPermissions: "Bypass permissions",
  untrusted: "Ask before commands",
  "on-request": "Ask when needed",
  never: "Never ask",
};

/** Status bar under the agent's terminal: model, effort, permissions and context use, with
 * everything else the agent reports on click. */
export function AgentStatus({ session }: { session: SessionInfo }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const a = session.agent;

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", esc);
    };
  }, [open]);

  if (!a?.model) return null;
  const usage = totalUsage(session.usage);
  const context = a.contextUsed !== null ? Math.round(a.contextUsed) : null;

  return (
    <div ref={ref} className="@container relative flex shrink-0 border-t border-line">
      <button
        onClick={() => setOpen((o) => !o)}
        title="Agent status"
        className={clsx(
          "flex h-6 min-w-0 flex-1 items-center gap-3 overflow-hidden px-3 text-[11px] whitespace-nowrap text-muted hover:bg-hover hover:text-fg",
          open && "bg-hover text-fg",
        )}
      >
        <span className="flex shrink-0 items-center gap-1.5">
          <Gauge size={12} />
          {a.model}
        </span>
        {a.effort && <span className="shrink-0">effort {a.effort}</span>}
        {a.permissionMode && (
          <span className="min-w-0 truncate">{PERMISSION_MODES[a.permissionMode] ?? a.permissionMode}</span>
        )}
        <span className="flex-1" />
        {usage && <span className="hidden @lg:inline">{formatTokens(usage.tokens)} tokens</span>}
        {a.costUsd !== null && <span className="hidden @md:inline">${a.costUsd.toFixed(2)}</span>}
        {context !== null && <ContextMeter percent={context} />}
      </button>
      <CopyContinue session={session} />
      {open && (
        <div data-overlay className="absolute right-2 bottom-full z-40 mb-1 w-64 rounded-lg border border-line bg-elevated p-3 text-xs shadow-xl">
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5">
            <Row label="Model">{a.model}</Row>
            {a.effort && <Row label="Effort">{a.effort}</Row>}
            {a.fastMode !== null && <Row label="Fast mode">{a.fastMode ? "On" : "Off"}</Row>}
            {a.thinking !== null && <Row label="Thinking">{a.thinking ? "On" : "Off"}</Row>}
            {a.permissionMode && (
              <Row label="Permissions">{PERMISSION_MODES[a.permissionMode] ?? a.permissionMode}</Row>
            )}
            {context !== null && (
              <Row label="Context">
                {context}%{a.contextSize ? ` of ${formatTokens(a.contextSize)}` : ""}
              </Row>
            )}
            {usage && <Row label="Tokens">{formatTokens(usage.tokens)}</Row>}
            {a.costUsd !== null && <Row label="Cost">${a.costUsd.toFixed(2)}</Row>}
            {a.outputStyle && a.outputStyle !== "default" && <Row label="Output style">{a.outputStyle}</Row>}
            {a.version && <Row label="Version">{a.version}</Row>}
          </dl>
          {context !== null && (
            <div className="mt-2.5 h-1 overflow-hidden rounded-full bg-hover">
              <div className={clsx("h-full rounded-full", meterColor(context))} style={{ width: `${context}%` }} />
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/** Copies the command that continues this conversation in the user's own terminal. */
function CopyContinue({ session }: { session: SessionInfo }) {
  const { copied, copy: copyText } = useCopy();
  // Fetched ahead, so the click can write the clipboard while it still counts as one.
  const { data: command, error } = useQuery({
    queryKey: ["continueCommand", session.id],
    queryFn: () => api.sessionContinueCommand(session.id),
    staleTime: Infinity,
    retry: false,
  });
  function copy() {
    if (!command) {
      toast(error ? errorMessage(error) : "Not ready yet, try again in a moment");
      return;
    }
    copyText(command);
  }
  return (
    <button
      onClick={copy}
      aria-label="Copy command to continue elsewhere"
      title={
        copied
          ? "Copied"
          : "Copy the command that continues this conversation in your own terminal, where it ran. Close it in worki first, so only one Claude Code writes to it."
      }
      className="flex h-6 w-7 shrink-0 items-center justify-center border-l border-line text-muted hover:bg-hover hover:text-fg"
    >
      {copied ? <Check size={11} className="text-added" /> : <Copy size={11} />}
    </button>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-faint">{label}</dt>
      <dd className="truncate text-right text-fg">{children}</dd>
    </>
  );
}

function ContextMeter({ percent }: { percent: number }) {
  return (
    <span className="flex shrink-0 items-center gap-1.5" title={`${percent}% of the context window in use`}>
      context
      <span className="h-1 w-12 overflow-hidden rounded-full bg-hover">
        <span className={clsx("block h-full rounded-full", meterColor(percent))} style={{ width: `${percent}%` }} />
      </span>
      {percent}%
    </span>
  );
}

const meterColor = (percent: number) => (percent >= 80 ? "bg-removed" : percent >= 60 ? "bg-warn" : "bg-accent");
