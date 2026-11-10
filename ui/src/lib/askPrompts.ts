import { useUi } from "./store";

export type AskPrompt = "commit" | "resolveConflicts" | "pullRequest";

export const ASK_PROMPTS: Record<AskPrompt, { label: string; hint: string; text: string }> = {
  commit: {
    label: "Commit",
    hint: "Sent from the commit box.",
    text: "Commit the current uncommitted changes in this worktree. Group them into sensible commits with clear messages.",
  },
  resolveConflicts: {
    label: "Resolve rebase conflicts",
    hint: "{base} is the base branch, {files} the conflicting files.",
    text: "Rebase this branch onto {base} and resolve the conflicts (in {files}), keeping both sides' intent. Run the tests afterwards.",
  },
  pullRequest: {
    label: "Pull request title and description",
    hint: "{base} is the base branch. worki adds the branch's commits and diff, and asks for the answer in a form it can fill in.",
    text: "Write a GitHub pull request title and description for the changes below, which this branch makes compared with {base}. Title: imperative mood, under 72 characters. Description: Markdown with a short summary of what changes and why, then a bullet list of the notable changes, and anything reviewers should test.",
  },
};

/** The user's prompt for an "Ask agent" task, or the default, with `{name}` placeholders filled in. */
export function askPrompt(kind: AskPrompt, vars: Record<string, string> = {}): string {
  const text = useUi.getState().askPrompts[kind]?.trim() || ASK_PROMPTS[kind].text;
  return text.replace(/\{(\w+)\}/g, (all, name: string) => vars[name] ?? all);
}
