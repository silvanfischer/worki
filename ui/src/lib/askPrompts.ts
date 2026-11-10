import { useUi } from "./store";

export type AskPrompt = "commitMessage" | "resolveConflicts" | "pullRequest";

export const ASK_PROMPTS: Record<AskPrompt, { label: string; hint: string; text: string }> = {
  commitMessage: {
    label: "Commits",
    hint: "{files} lists the checked files, {diff} is their diff and {log} the latest commit messages. worki adds how to answer at the end.",
    text: "Split the changes below into commits and write their messages, in the style of these recent commit messages:\n{log}\n\nUse one commit unless the changes are clearly separate (unrelated fixes, a refactoring next to a feature); then make one commit per change. Subject: imperative mood, under 72 characters. Add a short body only when the reason for the change isn't obvious from the subject.\n\nFiles:\n{files}\n\nChanges:\n{diff}",
  },
  resolveConflicts: {
    label: "Resolve rebase conflicts",
    hint: "Sent to the worktree's agent. {base} is the base branch, {files} the conflicting files.",
    text: "Rebase this branch onto {base} and resolve the conflicts (in {files}), keeping both sides' intent. Run the tests afterwards.",
  },
  pullRequest: {
    label: "Pull request title and description",
    hint: "{base} is the base branch, {commits} the branch's commits, {files} the changed files and {diff} the diff. worki adds how to answer at the end.",
    text: "Write a GitHub pull request title and description for the changes this branch makes compared with {base}. Title: imperative mood, under 72 characters. Description: Markdown with a short summary of what changes and why, then a bullet list of the notable changes, and anything reviewers should test.\n\nCommits:\n{commits}\n\nChanged files:\n{files}\n\nDiff:\n{diff}",
  },
};

/** The user's prompt for an "Ask agent" task, or the default, with `{name}` placeholders filled in. */
export function askPrompt(kind: AskPrompt, vars: Record<string, string> = {}): string {
  const text = useUi.getState().askPrompts[kind]?.trim() || ASK_PROMPTS[kind].text;
  return text.replace(/\{(\w+)\}/g, (all, name: string) => vars[name] ?? all);
}
