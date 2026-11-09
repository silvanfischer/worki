import clsx from "clsx";

import type { Activity, SessionInfo } from "../lib/api";

type DotState = Activity | "exited";

export const stateLabel: Record<DotState, string> = {
  waiting: "Waiting for you",
  working: "Working",
  idle: "Idle",
  exited: "Exited",
};

/** Most urgent state among sessions: waiting beats working beats idle. */
export function summarize(sessions: SessionInfo[]): DotState | null {
  const running = sessions.filter((s) => s.state.kind === "running");
  if (running.some((s) => s.activity === "waiting")) return "waiting";
  if (running.some((s) => s.activity === "working")) return "working";
  if (running.length > 0) return "idle";
  return sessions.length > 0 ? "exited" : null;
}

export function sessionState(s: SessionInfo): DotState {
  return s.state.kind === "running" ? s.activity : "exited";
}

export function ActivityDot({ state, title }: { state: DotState; title?: string }) {
  return (
    <span className="relative flex size-2.5 shrink-0 items-center justify-center" title={title ?? stateLabel[state]}>
      {state === "waiting" && <span className="absolute inline-flex size-full animate-ping rounded-full bg-warn opacity-75" />}
      {state === "working" && <span className="absolute inline-flex size-full animate-pulse rounded-full bg-added/40 blur-[3px]" />}
      <span
        className={clsx(
          "relative inline-flex size-2 rounded-full",
          state === "waiting" && "bg-warn shadow-[0_0_6px_2px_var(--warn),0_0_14px_4px_var(--warn)]",
          state === "working" && "animate-pulse bg-added shadow-[0_0_6px_2px_var(--added),0_0_14px_4px_var(--added)]",
          state === "idle" && "bg-added/45",
          state === "exited" && "bg-faint",
        )}
      />
    </span>
  );
}
