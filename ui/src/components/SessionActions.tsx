import { useQuery } from "@tanstack/react-query";
import { openUrl } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import { Check, Copy, ExternalLink, LoaderCircle, PowerOff, Smartphone } from "lucide-react";

import { type AgentState, api, errorMessage, type SessionInfo } from "../lib/api";
import { toast } from "../lib/toast";
import { useCopy } from "../lib/useCopy";
import { Menu } from "./ui";

const iconButton =
  "relative flex size-6 shrink-0 items-center justify-center rounded-md text-muted hover:bg-hover hover:text-fg";

/** Turns Remote Control on, then offers its link on claude.ai/code. */
export function RemoteControl({
  session,
  remote,
}: {
  session: SessionInfo;
  remote: NonNullable<AgentState["remoteControl"]>;
}) {
  const { copied, copy } = useCopy();
  const turn = (on: boolean) =>
    api.changeChatSetting(session.id, { kind: "remoteControl", value: on }).catch((e) => toast(errorMessage(e)));
  const url = remote.url;
  if (remote.state === "off")
    return (
      <button
        onClick={() => turn(true)}
        aria-label="Turn on Remote Control"
        title="Turn on Remote Control, to continue this chat from claude.ai/code or the Claude app"
        className={iconButton}
      >
        <Smartphone size={12} />
      </button>
    );
  if (!url)
    return (
      <span title="Remote Control is connecting…" className={clsx(iconButton, "hover:bg-transparent hover:text-muted")}>
        <LoaderCircle size={12} className="animate-spin" />
      </span>
    );
  const reconnecting = remote.state === "reconnecting";
  return (
    <Menu
      above
      trigger={(toggle) => (
        <button
          onClick={toggle}
          aria-label="Remote Control"
          title={
            reconnecting
              ? "Remote Control lost its connection and is reconnecting. In Docker Sandboxes it only connects with: sbx settings set claude.remoteControl true"
              : copied
                ? "Link copied"
                : `Remote Control is on: ${url}`
          }
          className={clsx(iconButton, "text-fg")}
        >
          {copied ? <Check size={12} className="text-added" /> : <Smartphone size={12} />}
          <span
            className={clsx("absolute top-0.5 right-0.5 size-1.5 rounded-full", reconnecting ? "bg-warn" : "bg-added")}
          />
        </button>
      )}
      items={[
        { label: "Open on claude.ai", icon: <ExternalLink size={13} />, onSelect: () => openUrl(url) },
        { label: "Copy link", icon: <Copy size={13} />, onSelect: () => copy(url) },
        { label: "Turn off", icon: <PowerOff size={13} />, danger: true, onSelect: () => turn(false) },
      ]}
    />
  );
}

/** Copies the command that continues this conversation in the user's own terminal. */
export function CopyContinue({ session, className }: { session: SessionInfo; className: string }) {
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
      className={className}
    >
      {copied ? <Check size={11} className="text-added" /> : <Copy size={11} />}
    </button>
  );
}
