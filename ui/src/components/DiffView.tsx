import clsx from "clsx";
import { MessageSquarePlus, Send, Trash2, X } from "lucide-react";
import { Fragment, useState } from "react";

import { type DiffLine, parseDiff } from "../lib/diff";
import { errorMessage } from "../lib/api";
import { useFileDiff, useSessions } from "../lib/queries";
import {
  type ReviewComment,
  reviewPrompt,
  reviewTarget,
  sendReview,
  useReview,
  useWorktreeComments,
} from "../lib/review";
import { useUi } from "../lib/store";
import { Button, DiffStat, ErrorText, IconButton, Textarea } from "./ui";

/** Keeps comment rows in view while the code beside them scrolls sideways. */
const PINNED = "sticky left-0 w-[100cqw] px-3";

type CodeLine = Extract<DiffLine, { kind: "context" | "add" | "del" }>;

function anchor(l: CodeLine): { side: "new" | "old"; line: number } {
  return l.kind === "del" ? { side: "old", line: l.oldNo! } : { side: "new", line: l.newNo! };
}

export function DiffView({
  worktreePath,
  file,
  base,
  onClose,
}: {
  worktreePath: string;
  file: string;
  base: string | null;
  onClose: () => void;
}) {
  const { data, error, isLoading } = useFileDiff(worktreePath, file, base);
  const comments = useWorktreeComments(worktreePath).filter((c) => c.file === file);
  const addComment = useReview((s) => s.add);
  const [drafting, setDrafting] = useState<string | null>(null);
  const { data: sessions = [] } = useSessions();
  const activeId = useUi((s) => s.activeSession[worktreePath]);
  const setActive = useUi((s) => s.setActiveSession);
  const target = reviewTarget(
    sessions.filter((s) => s.worktreePath === worktreePath),
    activeId,
  );
  const parsed = data !== undefined ? parseDiff(data) : null;
  const added = parsed?.lines.filter((l) => l.kind === "add").length ?? 0;
  const removed = parsed?.lines.filter((l) => l.kind === "del").length ?? 0;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line px-2">
        <span className="selectable min-w-0 flex-1 truncate pl-1 font-mono text-xs">{file}</span>
        {base && <span className="text-[11px] text-faint">vs {base}</span>}
        {parsed && <DiffStat insertions={added} deletions={removed} />}
        <IconButton label="Close diff" onClick={onClose}>
          <X size={14} />
        </IconButton>
      </div>
      <div className="selectable @container min-h-0 flex-1 overflow-auto font-mono text-xs leading-[1.6]">
        {isLoading && <p className="p-3 text-faint">Loading diff…</p>}
        {error && (
          <div className="p-3">
            <ErrorText>{String(error)}</ErrorText>
          </div>
        )}
        {parsed?.binary && <p className="p-3 text-faint">Binary file changed.</p>}
        {parsed && !parsed.binary && parsed.lines.length === 0 && (
          <p className="p-3 text-faint">No changes in this file anymore.</p>
        )}
        {parsed && parsed.lines.length > 0 && (
          <table className="w-full border-collapse">
            <tbody>
              {parsed.lines.map((l, i) => {
                if (l.kind === "hunk")
                  return (
                    <tr key={i} className="bg-accent/10 text-muted">
                      <td colSpan={3} className="px-3 py-0.5">
                        {l.text}
                      </td>
                    </tr>
                  );
                if (l.kind === "note")
                  return (
                    <tr key={i} className="text-faint">
                      <td colSpan={2} />
                      <td className="px-3 italic">{l.text}</td>
                    </tr>
                  );
                const at = anchor(l);
                const key = `${at.side}:${at.line}`;
                const here = comments.filter((c) => c.side === at.side && c.line === at.line);
                return (
                  <Fragment key={i}>
                    <tr
                      className={clsx(
                        "group",
                        l.kind === "add" && "bg-added/12",
                        l.kind === "del" && "bg-removed/12",
                      )}
                    >
                      <td className="w-12 border-r border-line px-2 text-right text-faint select-none">
                        {l.oldNo ?? ""}
                      </td>
                      <td className="relative w-12 border-r border-line px-2 text-right text-faint select-none">
                        <button
                          title="Comment on this line"
                          onClick={() => setDrafting(key)}
                          className="absolute top-0 -left-1 hidden size-[19px] items-center justify-center rounded bg-accent text-accent-fg group-hover:flex"
                        >
                          <MessageSquarePlus size={11} />
                        </button>
                        {l.newNo ?? ""}
                      </td>
                      <td className="px-3 whitespace-pre">
                        <span
                          className={clsx(
                            "mr-2 select-none",
                            l.kind === "add" ? "text-added" : l.kind === "del" ? "text-removed" : "text-faint",
                          )}
                        >
                          {l.kind === "add" ? "+" : l.kind === "del" ? "−" : " "}
                        </span>
                        {l.text}
                      </td>
                    </tr>
                    {here.map((c) => (
                      <CommentRow key={c.id} worktreePath={worktreePath} comment={c} />
                    ))}
                    {drafting === key && (
                      <tr>
                        <td colSpan={3} className="border-y border-line bg-panel py-2 font-sans">
                          <div className={PINNED}>
                            <CommentEditor
                              initial=""
                              submitLabel="Add comment"
                              sendTo={target?.title}
                              onSend={async (text) => {
                                await sendReview(target!.id, reviewPrompt([{ id: "", file, ...at, code: l.text, text }]));
                                setActive(worktreePath, target!.id);
                                setDrafting(null);
                              }}
                              onCancel={() => setDrafting(null)}
                              onSubmit={(text) => {
                                addComment(worktreePath, { file, ...at, code: l.text, text });
                                setDrafting(null);
                              }}
                            />
                          </div>
                        </td>
                      </tr>
                    )}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        )}
        {parsed && <OrphanComments worktreePath={worktreePath} lines={parsed.lines} comments={comments} />}
      </div>
    </div>
  );
}

function CommentRow({ worktreePath, comment }: { worktreePath: string; comment: ReviewComment }) {
  const update = useReview((s) => s.update);
  const remove = useReview((s) => s.remove);
  const [editing, setEditing] = useState(false);
  return (
    <tr>
      <td colSpan={3} className="border-y border-line bg-panel py-2 font-sans">
        <div className={PINNED}>
          {editing ? (
            <CommentEditor
              initial={comment.text}
              submitLabel="Save"
              onCancel={() => setEditing(false)}
              onSubmit={(text) => {
                update(worktreePath, comment.id, text);
                setEditing(false);
              }}
            />
          ) : (
            <div className="flex items-start gap-2">
              <p className="flex-1 cursor-text text-[13px] whitespace-pre-wrap" onClick={() => setEditing(true)}>
                {comment.text}
              </p>
              <IconButton label="Delete comment" onClick={() => remove(worktreePath, comment.id)}>
                <Trash2 size={12} />
              </IconButton>
            </div>
          )}
        </div>
      </td>
    </tr>
  );
}

/** Comments whose line is no longer in the diff, e.g. after the agent changed the file. */
function OrphanComments({
  worktreePath,
  lines,
  comments,
}: {
  worktreePath: string;
  lines: DiffLine[];
  comments: ReviewComment[];
}) {
  const shown = new Set(
    lines.flatMap((l) => (l.kind === "context" || l.kind === "add" || l.kind === "del" ? [anchor(l)] : [])).map(
      (a) => `${a.side}:${a.line}`,
    ),
  );
  const orphans = comments.filter((c) => !shown.has(`${c.side}:${c.line}`));
  if (orphans.length === 0) return null;
  return (
    <div className="border-t border-line p-3 font-sans">
      <p className="mb-2 text-[11px] font-semibold tracking-wide text-faint uppercase">Comments on lines not in the diff</p>
      <table className="w-full">
        <tbody>
          {orphans.map((c) => (
            <Fragment key={c.id}>
              <tr>
                <td className="font-mono text-xs text-faint">
                  {c.side === "old" ? "removed line " : "line "}
                  {c.line}: {c.code.trim()}
                </td>
              </tr>
              <CommentRow worktreePath={worktreePath} comment={c} />
            </Fragment>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function CommentEditor({
  initial,
  submitLabel,
  sendTo,
  onSubmit,
  onSend,
  onCancel,
}: {
  initial: string;
  submitLabel: string;
  sendTo?: string;
  onSubmit: (text: string) => void;
  onSend?: (text: string) => Promise<void>;
  onCancel: () => void;
}) {
  const [text, setText] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const submit = () => text.trim() && onSubmit(text.trim());
  const send = () => {
    if (!text.trim() || !onSend) return;
    setError(null);
    onSend(text.trim()).catch((e) => setError(errorMessage(e)));
  };
  return (
    <div className="flex flex-col gap-2">
      <Textarea
        autoFocus
        rows={3}
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder="What should the agent change here?"
        className="font-sans text-[13px]"
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) submit();
          if (e.key === "Escape") onCancel();
        }}
      />
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        {onSend && (
          <Button
            onClick={send}
            disabled={!text.trim() || !sendTo}
            title={sendTo ? `Send only this comment to ${sendTo} now` : "Start an agent in this worktree to send comments"}
          >
            <Send size={12} />
            Send now
          </Button>
        )}
        <Button variant="primary" onClick={submit} disabled={!text.trim()}>
          {submitLabel}
        </Button>
      </div>
      <ErrorText>{error}</ErrorText>
    </div>
  );
}
