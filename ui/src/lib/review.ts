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
  /** Set for a pull request check that failed: its name, with the error in `text` instead of a
   * comment on a diff line. */
  check?: string;
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
  const checks: ReviewComment[] = [];
  for (const c of comments) {
    if (c.check) checks.push(c);
    else byFile.set(c.file, [...(byFile.get(c.file) ?? []), c]);
  }
  const parts: string[] = [];
  if (byFile.size > 0) parts.push("Address these review comments on your changes:");
  for (const [file, list] of byFile) {
    for (const c of [...list].sort((a, b) => a.line - b.line)) {
      const where = c.side === "old" ? `${file}, removed line ${c.line}` : `${file}:${c.line}`;
      parts.push(`${where}\n> ${c.code.trim()}\n${c.text.trim()}`);
    }
  }
  if (checks.length > 0) parts.push("These checks failed on the pull request. Find the cause and fix it:");
  for (const c of checks) parts.push(`Check "${c.check}" failed:\n\`\`\`\n${c.text.trim()}\n\`\`\``);
  return parts.join("\n\n");
}

/** The agent session a review goes to: the focused one if it's a running agent, else the newest
 * running agent. */
export function reviewTarget(sessions: SessionInfo[], activeId: string | undefined): SessionInfo | undefined {
  const agents = sessions.filter((s) => s.profileId && s.state.kind === "running");
  return agents.find((s) => s.id === activeId) ?? agents[agents.length - 1];
}

/** Sends the review to the agent: as a message to a chat, or pasted into a terminal's prompt and
 * submitted (bracketed paste keeps the newlines from submitting early). */
export async function sendReview(session: SessionInfo, text: string) {
  if (session.kind === "chat") {
    await api.sendChatMessage(session.id, text);
    return;
  }
  await api.writeSession(session.id, `\x1b[200~${text}\x1b[201~`);
  await new Promise((r) => setTimeout(r, 150));
  await api.writeSession(session.id, "\r");
}
