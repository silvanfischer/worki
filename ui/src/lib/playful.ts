import { useEffect, useRef, useState } from "react";

import type { SessionInfo } from "./api";
import { useUi } from "./store";

export const usePlayful = () => useUi((s) => s.playful);

export interface AgentIdentity {
  name: string;
  color: string;
}

const NAMES = ["Otter", "Fox", "Octo", "Owl", "Bee", "Turtle", "Hedgehog", "Penguin", "Koala", "Frog", "Panda", "Whale"];
const COLORS = ["#14b8a6", "#f97316", "#ec4899", "#8b5cf6", "#0ea5e9", "#84cc16", "#f59e0b", "#f43f5e"];

function hash(text: string) {
  let h = 2166136261;
  for (const c of text) h = Math.imul(h ^ c.charCodeAt(0), 16777619);
  return h >>> 0;
}

/** A stable name and colour for an agent session, so parallel agents are easy to tell apart. */
export function agentIdentity(session: Pick<SessionInfo, "id">): AgentIdentity {
  const h = hash(session.id);
  return { name: NAMES[h % NAMES.length], color: COLORS[Math.floor(h / NAMES.length) % COLORS.length] };
}

export const isAgent = (session: SessionInfo) => session.profileId !== null;

/** The session's identity when it's an agent and the playful look is on. */
export const identityOf = (session: SessionInfo, playful: boolean) =>
  playful && isAgent(session) ? agentIdentity(session) : null;

/** True for `ms` right after `value` changed from `from` to `to`. */
export function useJustChanged<T>(value: T, from: T, to: T, ms: number) {
  const previous = useRef(value);
  const [changed, setChanged] = useState(false);
  useEffect(() => {
    const before = previous.current;
    previous.current = value;
    if (before !== from || value !== to) return;
    setChanged(true);
    const timer = setTimeout(() => setChanged(false), ms);
    return () => {
      clearTimeout(timer);
      setChanged(false);
    };
  }, [value, from, to, ms]);
  return changed;
}

const TIPS = [
  "⌘K jumps to any worktree or action.",
  "Paste a screenshot into a chat to show the agent what you mean.",
  "⌃` opens your own shell below, outside the sandbox.",
  "⌘J shows what the agent changed.",
  "Comment on a diff line and send all comments as one review.",
  "Save a snapshot once, and new worktrees start with everything installed.",
];

/** One of the tips, changing every minute. */
export const tipOfTheMoment = () => TIPS[Math.floor(Date.now() / 60_000) % TIPS.length];
