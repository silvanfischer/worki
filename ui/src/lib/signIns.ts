import { create } from "zustand";

import { type AppEvents, useTauriEvent } from "./events";

/** A device-code sign-in an agent waits for before it starts in a worktree. */
export type PendingSignIn = Omit<AppEvents["agent-sign-in"], "worktreePath">;

export const useSignIns = create<{ byWorktree: Record<string, PendingSignIn> }>()(() => ({ byWorktree: {} }));

function setSignIn(path: string, signIn: PendingSignIn | null) {
  useSignIns.setState((s) => {
    const byWorktree = { ...s.byWorktree };
    if (signIn) byWorktree[path] = signIn;
    else delete byWorktree[path];
    return { byWorktree };
  });
}

/** Keeps `useSignIns` in step with the sign-ins worki runs. */
export function useFollowSignIns() {
  useTauriEvent("agent-sign-in", ({ worktreePath, ...signIn }) => setSignIn(worktreePath, signIn));
  useTauriEvent("agent-signed-in", ({ worktreePath }) => setSignIn(worktreePath, null));
}
