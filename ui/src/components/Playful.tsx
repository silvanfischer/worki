import clsx from "clsx";
import { Check } from "lucide-react";

import type { SessionInfo } from "../lib/api";
import { agentIdentity, useJustChanged } from "../lib/playful";
import { sessionState, stateLabel } from "./ActivityDot";

/** The session's own colour, so parallel agents are easy to tell apart, with the usual state. */
export function AgentDot({ session }: { session: SessionInfo }) {
  const { name, color } = agentIdentity(session);
  const state = sessionState(session);
  const done = useJustChanged(state, "working", "idle", 1800);
  const title = session.attention ? `${name} is waiting: ${session.attention}` : `${name}: ${stateLabel[state]}`;
  if (done) {
    return (
      <span title={`${name} is done`} className="flex size-2.5 shrink-0 items-center justify-center text-added">
        <Check size={11} strokeWidth={3.5} />
      </span>
    );
  }
  return (
    <span title={title} className="relative flex size-2.5 shrink-0 items-center justify-center">
      {state === "working" && (
        <span
          className="absolute inline-flex size-full rounded-full blur-[3px] motion-safe:animate-pulse"
          style={{ background: `color-mix(in srgb, ${color} 45%, transparent)` }}
        />
      )}
      {state === "waiting" && (
        <span className="absolute inline-flex size-full rounded-full bg-warn opacity-60 motion-safe:animate-ping" />
      )}
      <span
        className={clsx(
          "relative inline-flex size-2 rounded-full",
          state === "working" && "motion-safe:animate-pulse",
          state === "idle" && "opacity-55",
          state === "exited" && "opacity-30",
        )}
        style={{
          background: color,
          boxShadow:
            state === "waiting"
              ? "0 0 0 1.5px var(--panel), 0 0 0 3px var(--warn)"
              : state === "working"
                ? `0 0 6px 2px ${color}, 0 0 14px 4px ${color}`
                : undefined,
        }}
      />
    </span>
  );
}

/** A small tree with bare branches, for a project without worktrees. */
export function TreeDoodle() {
  return (
    <svg width="96" height="80" viewBox="0 0 96 80" fill="none" className="text-muted" aria-hidden>
      <path d="M8 74h80" stroke="currentColor" strokeWidth="2" strokeLinecap="round" opacity=".4" />
      <path
        d="M48 74V40m0 14-14-10m14 2 13-11M34 44l-8-9m8 9-2-12m29 1 6-10m-6 10 10-2"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path d="M48 40V24" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" />
      <circle cx="48" cy="18" r="6" fill="var(--added)" opacity=".85" />
      <circle cx="26" cy="33" r="3.5" fill="var(--accent)" opacity=".8" />
      <circle cx="67" cy="21" r="3.5" fill="#8250df" opacity=".8" />
    </svg>
  );
}
