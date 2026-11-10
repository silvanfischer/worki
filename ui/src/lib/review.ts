import { create } from "zustand";
import { persist } from "zustand/middleware";

import { api, type SessionInfo } from "./api";

export interface ReviewComment {
  id: string;
  file: string;
  /** Which side of the diff the line is on: the new file, or a removed line of the old one. */
  side: "new" | "old";
  line: number;
  code: string;
  text: string;
}

interface ReviewState {
  /** Draft comments per worktree path. */
  comments: Record<string, ReviewComment[]>;
  add: (worktree: string, comment: Omit<ReviewComment, "id">) => void;
  update: (worktree: string, id: string, text: string) => void;
  remove: (worktree: string, id: string) => void;
  clear: (worktree: string) => void;
}

export const useReview = create<ReviewState>()(
  persist(
    (set) => {
      const edit = (worktree: string, f: (list: ReviewComment[]) => ReviewComment[]) =>
        set((s) => ({ comments: { ...s.comments, [worktree]: f(s.comments[worktree] ?? []) } }));
      return {
        comments: {},
        add: (worktree, c) => edit(worktree, (list) => [...list, { ...c, id: crypto.randomUUID() }]),
        update: (worktree, id, text) => edit(worktree, (list) => list.map((c) => (c.id === id ? { ...c, text } : c))),
        remove: (worktree, id) => edit(worktree, (list) => list.filter((c) => c.id !== id)),
        clear: (worktree) =>
          set((s) => {
            const { [worktree]: _, ...rest } = s.comments;
            return { comments: rest };
          }),
      };
    },
    { name: "worki-review" },
  ),
);

const NO_COMMENTS: ReviewComment[] = [];

export const useWorktreeComments = (worktree: string) => useReview((s) => s.comments[worktree] ?? NO_COMMENTS);

export function reviewPrompt(comments: ReviewComment[]): string {
  const byFile = new Map<string, ReviewComment[]>();
  for (const c of comments) byFile.set(c.file, [...(byFile.get(c.file) ?? []), c]);
  const parts = ["Address these review comments on your changes:"];
  for (const [file, list] of byFile) {
    for (const c of [...list].sort((a, b) => a.line - b.line)) {
      const where = c.side === "old" ? `${file}, removed line ${c.line}` : `${file}:${c.line}`;
      parts.push(`${where}\n> ${c.code.trim()}\n${c.text.trim()}`);
    }
  }
  return parts.join("\n\n");
}

/** The agent session a review goes to: the focused one if it's a running agent, else any running agent. */
export function reviewTarget(sessions: SessionInfo[], activeId: string | undefined): SessionInfo | undefined {
  const agents = sessions.filter((s) => s.profileId && s.state.kind === "running");
  return agents.find((s) => s.id === activeId) ?? agents[agents.length - 1];
}

/** Pastes the review into the agent's prompt and submits it. Bracketed paste keeps the newlines
 * from submitting early. */
export async function sendReview(sessionId: string, text: string) {
  await api.writeSession(sessionId, `\x1b[200~${text}\x1b[201~`);
  await new Promise((r) => setTimeout(r, 150));
  await api.writeSession(sessionId, "\r");
}
